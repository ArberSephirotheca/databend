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

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

use common_meta_types::RoleInfo;

// An role can be granted with multiple roles, find all the related roles in a DFS manner
pub fn find_all_related_roles(
    cache: &HashMap<String, RoleInfo>,
    role_identities: &[String],
) -> Vec<RoleInfo> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut result: Vec<RoleInfo> = vec![];
    let mut q: VecDeque<String> = role_identities.iter().cloned().collect();
    while let Some(role_identity) = q.pop_front() {
        if visited.contains(&role_identity) {
            continue;
        }
        let cache_key = role_identity.to_string();
        visited.insert(role_identity);
        let role = match cache.get(&cache_key) {
            None => continue,
            Some(role) => role,
        };
        result.push(role.clone());
        for related_role in role.grants.roles() {
            q.push_back(related_role);
        }
    }
    result
}
