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
use std::fmt::Debug;
use std::fmt::Formatter;
use std::sync::Arc;

use common_expression::DataSchemaRef;
use common_meta_types::UserStageInfo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateStagePlan {
    pub if_not_exists: bool,
    pub tenant: String,
    pub user_stage_info: UserStageInfo,
}

impl CreateStagePlan {
    pub fn schema(&self) -> DataSchemaRef {
        Arc::new(DataSchema::empty())
    }
}

/// Drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropStagePlan {
    pub if_exists: bool,
    pub name: String,
}

impl DropStagePlan {
    pub fn schema(&self) -> DataSchemaRef {
        Arc::new(DataSchema::empty())
    }
}

/// Remove.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoveStagePlan {
    pub stage: UserStageInfo,
    pub path: String,
    pub pattern: String,
}

impl RemoveStagePlan {
    pub fn schema(&self) -> DataSchemaRef {
        Arc::new(DataSchema::empty())
    }
}
