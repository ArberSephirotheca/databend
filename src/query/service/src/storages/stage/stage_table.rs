// Copyright 2021 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::any::Any;
use std::str::FromStr;
use std::sync::Arc;

use common_datablocks::DataBlock;
use common_exception::ErrorCode;
use common_exception::Result;
use common_formats::output_format::OutputFormatType;
use common_legacy_planners::Extras;
use common_legacy_planners::Partitions;
use common_legacy_planners::ReadDataSourcePlan;
use common_legacy_planners::StageTableInfo;
use common_legacy_planners::Statistics;
use common_meta_app::schema::TableInfo;
use common_meta_types::StageType;
use common_meta_types::UserStageInfo;
use common_pipeline_core::processors::port::InputPort;
use common_pipeline_core::processors::port::OutputPort;
use common_pipeline_core::SinkPipeBuilder;
use common_pipeline_sources::processors::sources::input_formats::InputContext;
use common_storage::init_operator;
use opendal::Operator;
use parking_lot::Mutex;
use tracing::info;

use super::stage_table_sink::StageTableSink;
use crate::pipelines::processors::ContextSink;
use crate::pipelines::processors::TransformLimit;
use crate::pipelines::Pipeline;
use crate::sessions::TableContext;
use crate::storages::Table;

pub struct StageTable {
    table_info: StageTableInfo,
    // This is no used but a placeholder.
    // But the Table trait need it:
    // fn get_table_info(&self) -> &TableInfo).
    table_info_placeholder: TableInfo,
    input_context: Mutex<Option<Arc<InputContext>>>,
}

impl StageTable {
    pub fn try_create(table_info: StageTableInfo) -> Result<Arc<dyn Table>> {
        let table_info_placeholder = TableInfo::default().set_schema(table_info.schema());

        Ok(Arc::new(Self {
            table_info,
            table_info_placeholder,
            input_context: Default::default(),
        }))
    }

    fn get_input_context(&self) -> Option<Arc<InputContext>> {
        let guard = self.input_context.lock();
        guard.clone()
    }

    /// TODO: we should support construct operator with
    /// correct root.
    pub async fn get_op(ctx: &Arc<dyn TableContext>, stage: &UserStageInfo) -> Result<Operator> {
        if stage.stage_type == StageType::Internal {
            ctx.get_storage_operator()
        } else {
            Ok(init_operator(&stage.stage_params.storage)?)
        }
    }

    pub fn unload_path(&self, uuid: &str, group_id: usize, idx: usize) -> String {
        let format_name = format!(
            "{:?}",
            self.table_info.stage_info.file_format_options.format
        );
        if self.table_info.path.ends_with("data_") {
            format!(
                "{}{}_{}_{}.{}",
                self.table_info.path,
                uuid,
                group_id,
                idx,
                format_name.to_ascii_lowercase()
            )
        } else {
            format!(
                "{}/data_{}_{}_{}.{}",
                self.table_info.path,
                uuid,
                group_id,
                idx,
                format_name.to_ascii_lowercase()
            )
        }
    }
}

#[async_trait::async_trait]
impl Table for StageTable {
    fn as_any(&self) -> &dyn Any {
        self
    }

    // External stage has no table info yet.
    fn get_table_info(&self) -> &TableInfo {
        &self.table_info_placeholder
    }

    async fn read_partitions(
        &self,
        ctx: Arc<dyn TableContext>,
        _push_downs: Option<Extras>,
    ) -> Result<(Statistics, Partitions)> {
        let operator = StageTable::get_op(&ctx, &self.table_info.stage_info).await?;
        let input_ctx = Arc::new(
            InputContext::try_create_from_copy(
                operator,
                ctx.get_settings().clone(),
                ctx.get_format_settings()?,
                self.table_info.schema.clone(),
                self.table_info.stage_info.clone(),
                self.table_info.files.clone(),
                ctx.get_scan_progress(),
            )
            .await?,
        );
        info!("copy into {:?}", input_ctx);
        let mut guard = self.input_context.lock();
        *guard = Some(input_ctx);
        Ok((Statistics::default(), vec![]))
    }

    fn read2(
        &self,
        _ctx: Arc<dyn TableContext>,
        _plan: &ReadDataSourcePlan,
        pipeline: &mut Pipeline,
    ) -> Result<()> {
        let input_ctx = self.get_input_context().unwrap();
        input_ctx.format.exec_copy(input_ctx.clone(), pipeline)?;

        let limit = self.table_info.stage_info.copy_options.size_limit;
        if limit > 0 {
            pipeline.resize(1)?;
            pipeline.add_transform(|transform_input_port, transform_output_port| {
                TransformLimit::try_create(
                    Some(limit),
                    0,
                    transform_input_port,
                    transform_output_port,
                )
            })?;
        }
        Ok(())
    }

    fn append2(&self, ctx: Arc<dyn TableContext>, pipeline: &mut Pipeline, _: bool) -> Result<()> {
        let mut sink_pipeline_builder = SinkPipeBuilder::create();
        let single = self.table_info.stage_info.copy_options.single;

        // parallel compact unload, the partial block will flush into next operator
        if !single {
            for _ in 0..pipeline.output_len() {
                let input_port = InputPort::create();
                let output_port = OutputPort::create();

                sink_pipeline_builder.add_sink(
                    input_port.clone(),
                    StageTableSink::try_create(
                        input_port,
                        ctx.clone(),
                        self.table_info.clone(),
                        single,
                        op.clone(),
                        Some(output_port),
                    )?,
                );
            }
            pipeline.add_pipe(sink_pipeline_builder.finalize());
        }

        // final compact unload
        pipeline.resize(1)?;

        let mut sink_pipeline_builder = SinkPipeBuilder::create();
        let input_port = InputPort::create();
        let output_port = OutputPort::create();

        sink_pipeline_builder.add_sink(
            input_port.clone(),
            StageTableSink::try_create(
                input_port,
                ctx.clone(),
                self.table_info.clone(),
                single,
                op.clone(),
                None,
            )?,
        );

        Ok(())
    }

    // TODO use tmp file_name & rename to have atomic commit
    async fn commit_insertion(
        &self,
        ctx: Arc<dyn TableContext>,
        operations: Vec<DataBlock>,
        overwrite: bool,
    ) -> Result<()> {
        Ok(())
    }

    // Truncate the stage file.
    async fn truncate(&self, _ctx: Arc<dyn TableContext>, _: bool) -> Result<()> {
        Err(ErrorCode::UnImplement(
            "S3 external table truncate() unimplemented yet!",
        ))
    }
}
