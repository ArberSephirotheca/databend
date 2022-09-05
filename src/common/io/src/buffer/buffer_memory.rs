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

use std::io::Result;

use crate::buffer::BufferRead;

pub type MemoryReader<'a> = &'a [u8];

impl<'a> BufferRead for MemoryReader<'a> {
    fn working_buf(&self) -> &[u8] {
        self
    }

    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(self)
    }

    fn consume(&mut self, amt: usize) {
        *self = &self[amt..];
    }
}
