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

use std::iter::TrustedLen;
use std::sync::atomic::Ordering;

use common_arrow::arrow::bitmap::Bitmap;
use common_arrow::arrow::bitmap::MutableBitmap;
use common_exception::ErrorCode;
use common_exception::Result;
use common_expression::BlockEntry;
use common_expression::DataBlock;
use common_expression::Scalar;
use common_expression::Value;
use common_hashtable::HashtableEntryRefLike;
use common_hashtable::HashtableLike;

use crate::pipelines::processors::transforms::hash_join::desc::MarkerKind;
use crate::pipelines::processors::transforms::hash_join::desc::JOIN_MAX_BLOCK_SIZE;
use crate::pipelines::processors::transforms::hash_join::row::RowPtr;
use crate::pipelines::processors::transforms::hash_join::ProbeState;
use crate::pipelines::processors::JoinHashTable;
use crate::sql::plans::JoinType;

impl JoinHashTable {
    pub(crate) fn probe_left_join<
        'a,
        const WITH_OTHER_CONJUNCT: bool,
        H: HashtableLike<Value = Vec<RowPtr>>,
        IT,
    >(
        &self,
        hash_table: &H,
        probe_state: &mut ProbeState,
        keys_iter: IT,
        input: &DataBlock,
    ) -> Result<Vec<DataBlock>>
    where
        IT: Iterator<Item = &'a H::Key> + TrustedLen,
        H::Key: 'a,
    {
        let valids = &probe_state.valids;

        // The left join will return multiple data blocks of similar size
        let mut probed_num = 0;
        let mut probed_blocks = vec![];
        let mut probe_indexes_len = 0;
        let probe_indexes = &mut probe_state.probe_indexes;
        let mut probe_indexes_vec = Vec::new();
        // Collect each probe_indexes, used by non-equi conditions filter
        let mut local_build_indexes = Vec::with_capacity(JOIN_MAX_BLOCK_SIZE);
        let mut validity = MutableBitmap::with_capacity(JOIN_MAX_BLOCK_SIZE);

        let data_blocks = self.row_space.datablocks();
        let num_rows = data_blocks
            .iter()
            .fold(0, |acc, chunk| acc + chunk.num_rows());

        let mut row_state = match WITH_OTHER_CONJUNCT {
            true => vec![0; input.num_rows()],
            false => vec![],
        };

        let dummy_probed_rows = vec![RowPtr {
            row_index: 0,
            chunk_index: 0,
            marker: Some(MarkerKind::False),
        }];

        // Start to probe hash table
        for (i, key) in keys_iter.enumerate() {
            let probe_result_ptr = match self.hash_join_desc.from_correlated_subquery {
                true => hash_table.entry(key),
                false => self.probe_key(hash_table, key, valids, i),
            };

            let (validity_value, probed_rows) = match probe_result_ptr {
                None => (false, &dummy_probed_rows),
                Some(v) => (true, v.get()),
            };

            if self.hash_join_desc.join_type == JoinType::Full {
                let mut build_indexes = self.hash_join_desc.join_state.build_indexes.write();
                // dummy row ptr
                // here assume there is no RowPtr, which chunk_index is usize::MAX and row_index is usize::MAX
                match probe_result_ptr {
                    None => build_indexes.push(RowPtr {
                        chunk_index: usize::MAX,
                        row_index: usize::MAX,
                        marker: Some(MarkerKind::False),
                    }),
                    Some(_) => {
                        build_indexes.extend_from_slice(probed_rows);
                    }
                };
            }

            if self.hash_join_desc.join_type == JoinType::Single && probed_rows.len() > 1 {
                return Err(ErrorCode::Internal(
                    "Scalar subquery can't return more than one row",
                ));
            }

            if WITH_OTHER_CONJUNCT {
                row_state[i] += probed_rows.len() as u32;
            }

            if probed_num + probed_rows.len() < JOIN_MAX_BLOCK_SIZE {
                local_build_indexes.extend_from_slice(probed_rows);
                probe_indexes[probe_indexes_len] = (i as u32, probed_rows.len() as u32);
                probe_indexes_len += 1;
                probed_num += probed_rows.len();
                validity.extend_constant(probed_rows.len(), validity_value);
            } else {
                let mut index = 0_usize;
                let mut remain = probed_rows.len();

                while index < probed_rows.len() {
                    if probed_num + remain < JOIN_MAX_BLOCK_SIZE {
                        local_build_indexes.extend_from_slice(&probed_rows[index..]);
                        probe_indexes[probe_indexes_len] = (i as u32, remain as u32);
                        probe_indexes_len += 1;
                        probed_num += remain;
                        validity.extend_constant(remain, validity_value);

                        index += remain;
                    } else {
                        if self.interrupt.load(Ordering::Relaxed) {
                            return Err(ErrorCode::AbortedQuery(
                                "Aborted query, because the server is shutting down or the query was killed.",
                            ));
                        }

                        let addition = JOIN_MAX_BLOCK_SIZE - probed_num;
                        let new_index = index + addition;

                        local_build_indexes.extend_from_slice(&probed_rows[index..new_index]);
                        probe_indexes[probe_indexes_len] = (i as u32, addition as u32);
                        probe_indexes_len += 1;
                        probed_num += addition;
                        validity.extend_constant(addition, validity_value);

                        let validity_bitmap: Bitmap = validity.into();
                        let build_block = if !self.hash_join_desc.from_correlated_subquery
                            && self.hash_join_desc.join_type == JoinType::Single
                            && validity_bitmap.unset_bits() == input.num_rows()
                        {
                            // Uncorrelated scalar subquery and no row was returned by subquery
                            // We just construct a block with NULLs
                            let build_data_schema = self.row_space.data_schema.clone();
                            let columns = build_data_schema
                                .fields()
                                .iter()
                                .map(|field| BlockEntry {
                                    data_type: field.data_type().wrap_nullable(),
                                    value: Value::Scalar(Scalar::Null),
                                })
                                .collect::<Vec<_>>();
                            DataBlock::new(columns, input.num_rows())
                        } else {
                            self.row_space
                                .gather(&local_build_indexes, &data_blocks, &num_rows)?
                        };

                        // For left join, wrap nullable for build block
                        let (nullable_columns, num_rows) = if self.row_space.datablocks().is_empty()
                            && !local_build_indexes.is_empty()
                        {
                            (
                                build_block
                                    .columns()
                                    .iter()
                                    .map(|c| BlockEntry {
                                        value: Value::Scalar(Scalar::Null),
                                        data_type: c.data_type.wrap_nullable(),
                                    })
                                    .collect::<Vec<_>>(),
                                local_build_indexes.len(),
                            )
                        } else {
                            (
                                build_block
                                    .columns()
                                    .iter()
                                    .map(|c| {
                                        Self::set_validity(
                                            c,
                                            build_block.num_rows(),
                                            &validity_bitmap,
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                                validity_bitmap.len(),
                            )
                        };

                        let nullable_build_block = DataBlock::new(nullable_columns, num_rows);

                        // For full join, wrap nullable for probe block
                        let mut probe_block = DataBlock::take_compacted_indices(
                            input,
                            &probe_indexes[0..probe_indexes_len],
                            probed_num,
                        )?;
                        let num_rows = probe_block.num_rows();
                        if self.hash_join_desc.join_type == JoinType::Full {
                            let nullable_probe_columns = probe_block
                                .columns()
                                .iter()
                                .map(|c| {
                                    let mut probe_validity = MutableBitmap::new();
                                    probe_validity.extend_constant(num_rows, true);
                                    let probe_validity: Bitmap = probe_validity.into();
                                    Self::set_validity(c, num_rows, &probe_validity)
                                })
                                .collect::<Vec<_>>();
                            probe_block = DataBlock::new(nullable_probe_columns, num_rows);
                        }

                        let merged_block =
                            self.merge_eq_block(&nullable_build_block, &probe_block)?;

                        if !merged_block.is_empty() {
                            probe_indexes_vec.push(probe_indexes[0..probe_indexes_len].to_vec());
                            probed_blocks.push(merged_block);
                        }

                        index = new_index;
                        remain -= addition;

                        local_build_indexes.clear();
                        probe_indexes_len = 0;
                        probed_num = 0;
                        validity = MutableBitmap::with_capacity(JOIN_MAX_BLOCK_SIZE);
                    }
                }
            }
        }

        // For full join, wrap nullable for probe block
        let mut probe_block = DataBlock::take_compacted_indices(
            input,
            &probe_indexes[0..probe_indexes_len],
            probed_num,
        )?;
        if self.hash_join_desc.join_type == JoinType::Full {
            let nullable_probe_columns = probe_block
                .columns()
                .iter()
                .map(|c| {
                    let mut probe_validity = MutableBitmap::new();
                    probe_validity.extend_constant(probe_block.num_rows(), true);
                    let probe_validity: Bitmap = probe_validity.into();
                    Self::set_validity(c, probe_block.num_rows(), &probe_validity)
                })
                .collect::<Vec<_>>();
            probe_block = DataBlock::new(nullable_probe_columns, probe_block.num_rows());
        }

        if !WITH_OTHER_CONJUNCT {
            let mut rest_pairs = self.hash_join_desc.join_state.rest_pairs.write();
            rest_pairs.1.extend(local_build_indexes);
            rest_pairs.0.push(probe_block);
            let validity: Bitmap = validity.into();
            let mut validity_state = self.hash_join_desc.join_state.validity.write();
            validity_state.extend_from_bitmap(&validity);
            return Ok(probed_blocks);
        }

        let validity: Bitmap = validity.into();
        let build_block = if !self.hash_join_desc.from_correlated_subquery
            && self.hash_join_desc.join_type == JoinType::Single
            && validity.unset_bits() == input.num_rows()
        {
            // Uncorrelated scalar subquery and no row was returned by subquery
            // We just construct a block with NULLs
            let build_data_schema = self.row_space.data_schema.clone();
            let columns = build_data_schema
                .fields()
                .iter()
                .map(|field| BlockEntry {
                    data_type: field.data_type().wrap_nullable(),
                    value: Value::Scalar(Scalar::Null),
                })
                .collect::<Vec<_>>();
            DataBlock::new(columns, input.num_rows())
        } else {
            self.row_space
                .gather(&local_build_indexes, &data_blocks, &num_rows)?
        };

        // For left join, wrap nullable for build chunk
        let nullable_columns =
            if self.row_space.datablocks().is_empty() && !local_build_indexes.is_empty() {
                build_block
                    .columns()
                    .iter()
                    .map(|c| BlockEntry {
                        data_type: c.data_type.clone(),
                        value: Value::Scalar(Scalar::Null),
                    })
                    .collect::<Vec<_>>()
            } else {
                build_block
                    .columns()
                    .iter()
                    .map(|c| Self::set_validity(c, build_block.num_rows(), &validity))
                    .collect::<Vec<_>>()
            };
        let nullable_build_block = DataBlock::new(nullable_columns, validity.len());

        // Process no-equi conditions
        let merged_block = self.merge_eq_block(&nullable_build_block, &probe_block)?;

        if !merged_block.is_empty() || probed_blocks.is_empty() {
            probe_indexes_vec.push(probe_indexes[0..probe_indexes_len].to_vec());
            probed_blocks.push(merged_block);
        }

        self.non_equi_conditions_for_left_join(&probed_blocks, &probe_indexes_vec, &mut row_state)
    }

    // keep at least one index of the positive state and the null state
    // bitmap: [1, 0, 1] with row_state [2, 1], probe_index: [(0, 2), (1, 1)] => [0, 0, 1]
    // bitmap will be [1, 0, 1] -> [1, 0, 1] -> [1, 0, 1] -> [1, 0, 1]
    // row_state will be [2, 1] -> [2, 1] -> [1, 1] -> [1, 1]
    pub(crate) fn fill_null_for_left_join(
        &self,
        bm: &mut MutableBitmap,
        probe_indexes: &[(u32, u32)],
        row_state: &mut [u32],
    ) {
        let mut index = 0;
        for (row, cnt) in probe_indexes {
            let row = *row as usize;
            for _ in 0..*cnt {
                if row_state[row] == 0 {
                    bm.set(index, true);
                    index += 1;
                    continue;
                }

                if row_state[row] == 1 {
                    if !bm.get(index) {
                        bm.set(index, true)
                    }
                    index += 1;
                    continue;
                }

                if !bm.get(index) {
                    row_state[row] -= 1;
                }
                index += 1;
            }
        }
    }
}
