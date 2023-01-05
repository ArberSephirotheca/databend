// Copyright 2022 Datafuse Labs.
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

use common_exception::ErrorCode;
use common_exception::Result;
use common_expression::types::AnyType;
use common_expression::BlockEntry;
use common_expression::DataBlock;
use common_expression::TableField;
use common_expression::TableSchemaRef;
use common_expression::Value;

use crate::hive_partition::HivePartInfo;
use crate::utils::str_field_to_scalar;

#[derive(Debug, Clone)]
pub struct HivePartitionFiller {
    schema: TableSchemaRef,
    pub partition_fields: Vec<TableField>,
    pub projections: Vec<usize>,
}

impl HivePartitionFiller {
    pub fn create(
        schema: TableSchemaRef,
        partition_fields: Vec<TableField>,
        projections: Vec<usize>,
    ) -> Self {
        HivePartitionFiller {
            schema,
            partition_fields,
            projections,
        }
    }

    fn generate_value(
        &self,
        _num_rows: usize,
        value: String,
        field: &TableField,
    ) -> Result<Value<AnyType>> {
        let value = str_field_to_scalar(&value, &field.data_type().into())?;
        Ok(Value::Scalar(value))
    }

    fn extract_partition_values(&self, hive_part: &HivePartInfo) -> Result<Vec<String>> {
        let partition_map = hive_part.get_partition_map();

        let mut partition_values = vec![];
        for field in self.partition_fields.iter() {
            match partition_map.get(field.name()) {
                Some(v) => partition_values.push(v.to_string()),
                None => {
                    return Err(ErrorCode::TableInfoError(format!(
                        "could't find hive partition info :{}, hive partition maps:{:?}",
                        field.name(),
                        partition_map
                    )));
                }
            };
        }
        Ok(partition_values)
    }

    pub fn fill_data(
        &self,
        data_block: DataBlock,
        part: &HivePartInfo,
        origin_num_rows: usize,
    ) -> Result<DataBlock> {
        let data_values = self.extract_partition_values(part)?;

        // create column, create datafiled
        let mut num_rows = data_block.num_rows();
        if num_rows == 0 {
            num_rows = origin_num_rows;
        }

        let mut columns = vec![];
        let mut j = 0;

        for (i, field) in self.partition_fields.iter().enumerate() {
            let index = self.schema.index_of(field.name())?;
            let project_index = self.projections.iter().position(|x| *x == index).unwrap();

            while columns.len() < project_index {
                columns.push(data_block.columns()[j].clone());
                j += 1;
            }

            let value = &data_values[i];
            let column = self.generate_value(num_rows, value.clone(), field)?;
            columns.push(BlockEntry {
                data_type: field.data_type().into(),
                value: column,
            });
        }

        while j < data_block.num_columns() {
            columns.push(data_block.columns()[j].clone());
            j += 1;
        }

        Ok(DataBlock::new(columns, data_block.num_rows()))
    }
}
