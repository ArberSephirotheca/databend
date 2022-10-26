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

use std::hash::Hasher;
use std::hash::Hash;
use std::mem::MaybeUninit;

use super::FastHash;
use super::HashtableKeyable;

#[derive(Clone, Copy)]
pub struct KeysRef {
    pub length: usize,
    pub address: usize,
    pub hash: u64,
}

impl KeysRef {
    pub fn create(address: usize, length: usize) -> KeysRef {
        let hash = unsafe { std::slice::from_raw_parts(address as *const u8, length).fast_hash() };
        KeysRef {
            length,
            address,
            hash,
        }
    }
}

impl Eq for KeysRef {}

impl PartialEq for KeysRef {
    fn eq(&self, other: &Self) -> bool {
        if self.length != other.length {
            return false;
        }

        unsafe {
            let self_value = std::slice::from_raw_parts(self.address as *const u8, self.length);
            let other_value = std::slice::from_raw_parts(other.address as *const u8, other.length);
            self_value == other_value
        }
    }
}

unsafe impl HashtableKeyable for KeysRef {
    fn is_zero(this: &MaybeUninit<Self>) -> bool {
        unsafe { this.assume_init_ref().address == 0 }
    }

    fn equals_zero(this: &Self) -> bool {
        this.address == 0
    }

    fn hash(&self) -> u64 {
        self.hash
    }
}

impl Hash for KeysRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let self_value =
            unsafe { std::slice::from_raw_parts(self.address as *const u8, self.length) };
        self_value.hash(state);
    }
}
