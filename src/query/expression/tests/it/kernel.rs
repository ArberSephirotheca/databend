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

use std::io::Write;

use common_expression::Chunk;
use common_expression::Column;
use common_expression::ColumnFrom;
use common_expression::Value;
use goldenfile::Mint;

#[test]
pub fn test_pass() {
    let mut mint = Mint::new("tests/it/testdata");
    let mut file = mint.new_goldenfile("kernel-pass.txt").unwrap();

    for filter in vec![
        Column::from_data(vec![true, false, false, false, true]),
        Column::from_data_with_validity(vec![true, true, false, true, true], vec![
            false, true, true, false, false,
        ]),
        Column::from_data(vec!["a", "b", "", "", "c"]),
        Column::from_data(vec![0, 1, 2, 3, 0]),
    ] {
        run_filter(&mut file, filter, &[
            Column::from_data(vec![0i32, 1, 2, 3, -4]),
            Column::from_data_with_validity(vec![10u8, 11, 12, 13, 14], vec![
                false, true, false, false, false,
            ]),
            Column::Null { len: 5 },
            Column::from_data_with_validity(vec!["x", "y", "z", "a", "b"], vec![
                false, true, true, false, false,
            ]),
        ]);
    }

    run_concat(&mut file, vec![
        vec![
            Column::from_data(vec![0i32, 1, 2, 3, -4]),
            Column::from_data_with_validity(vec![10u8, 11, 12, 13, 14], vec![
                false, true, false, false, false,
            ]),
            Column::Null { len: 5 },
            Column::EmptyArray { len: 5 },
            Column::from_data_with_validity(vec!["x", "y", "z", "a", "b"], vec![
                false, true, true, false, false,
            ]),
        ],
        vec![
            Column::from_data(vec![5i32, 6]),
            Column::from_data_with_validity(vec![15u8, 16], vec![false, true]),
            Column::Null { len: 2 },
            Column::EmptyArray { len: 2 },
            Column::from_data_with_validity(vec!["x", "y"], vec![false, true]),
        ],
    ]);

    run_take(&mut file, &[0, 3, 1], &[
        Column::from_data(vec![0i32, 1, 2, 3, -4]),
        Column::from_data_with_validity(vec![10u8, 11, 12, 13, 14], vec![
            false, true, false, false, false,
        ]),
        Column::Null { len: 5 },
        Column::from_data_with_validity(vec!["x", "y", "z", "a", "b"], vec![
            false, true, true, false, false,
        ]),
    ]);

    run_scatter(
        &mut file,
        &[
            Column::from_data(vec![0i32, 1, 2, 3, -4]),
            Column::from_data_with_validity(vec![10, 11, 12, 13, 14], vec![
                false, true, false, false, false,
            ]),
            Column::Null { len: 5 },
            Column::from_data_with_validity(vec!["x", "y", "z", "a", "b"], vec![
                false, true, true, false, false,
            ]),
        ],
        &[0, 0, 1, 2, 1],
        3,
    );
}

fn run_filter(file: &mut impl Write, predicate: Column, columns: &[Column]) {
    let len = columns.get(0).map_or(1, |c| c.len());
    let columns = columns.iter().map(|c| Value::Column(c.clone())).collect();

    let chunk = Chunk::new(columns, len);

    let predicate = Value::Column(predicate);
    let result = chunk.clone().filter(&predicate);

    match result {
        Ok(result_chunk) => {
            writeln!(file, "Filter:         {predicate}").unwrap();
            writeln!(file, "Source:\n{chunk}").unwrap();
            writeln!(file, "Result:\n{result_chunk}").unwrap();
            write!(file, "\n\n").unwrap();
        }
        Err(err) => {
            writeln!(file, "error: {}\n", err.message()).unwrap();
        }
    }
}

fn run_concat(file: &mut impl Write, columns: Vec<Vec<Column>>) {
    let chunks: Vec<Chunk> = columns
        .iter()
        .map(|cs| {
            let num_rows = cs.get(0).map_or(1, |c| c.len());
            let cs = cs.iter().map(|c| Value::Column(c.clone())).collect();
            Chunk::new(cs, num_rows)
        })
        .collect();

    let result = Chunk::concat(&chunks);

    match result {
        Ok(result_chunk) => {
            for (i, c) in chunks.iter().enumerate() {
                writeln!(file, "Concat-Column {}:", i).unwrap();
                writeln!(file, "{:?}", c).unwrap();
            }
            writeln!(file, "Result:\n{result_chunk}").unwrap();
            write!(file, "\n\n").unwrap();
        }
        Err(err) => {
            writeln!(file, "error: {}\n", err.message()).unwrap();
        }
    }
}

fn run_take(file: &mut impl Write, indices: &[u32], columns: &[Column]) {
    let len = columns.get(0).map_or(1, |c| c.len());
    let columns = columns.iter().map(|c| Value::Column(c.clone())).collect();
    let chunk = Chunk::new(columns, len);

    let result = Chunk::take(chunk.clone(), indices);

    match result {
        Ok(result_chunk) => {
            writeln!(file, "Take:         {indices:?}").unwrap();
            writeln!(file, "Source:\n{chunk}").unwrap();
            writeln!(file, "Result:\n{result_chunk}").unwrap();
            write!(file, "\n\n").unwrap();
        }
        Err(err) => {
            writeln!(file, "error: {}\n", err.message()).unwrap();
        }
    }
}

fn run_scatter(file: &mut impl Write, columns: &[Column], indices: &[u32], scatter_size: usize) {
    let len = columns.get(0).map_or(1, |c| c.len());
    let columns = columns.iter().map(|c| Value::Column(c.clone())).collect();
    let chunk = Chunk::new(columns, len);

    let result = Chunk::scatter(&chunk, indices, scatter_size);

    match result {
        Ok(result_chunk) => {
            writeln!(file, "Scatter:         {indices:?}").unwrap();
            writeln!(file, "Source:\n{chunk}").unwrap();

            for (i, c) in result_chunk.iter().enumerate() {
                writeln!(file, "Result-{i}:\n{c}").unwrap();
            }
            write!(file, "\n\n").unwrap();
        }
        Err(err) => {
            writeln!(file, "error: {}\n", err.message()).unwrap();
        }
    }
}
