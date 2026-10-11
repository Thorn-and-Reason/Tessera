use tessera::data::{Column, ColumnId, ColumnKind, ColumnMetadata, Dataset, Measurement};

pub fn sample_dataset() -> Dataset {
    Dataset::new(
        vec![
            Column::text(
                ColumnId(1),
                "Participant",
                (1..=6).map(|n| Some(format!("P{n:02}"))).collect(),
            )
            .unwrap(),
            Column::numeric(
                ColumnId(2),
                "Age",
                vec![
                    Some(17.0),
                    Some(18.0),
                    Some(20.0),
                    Some(21.0),
                    Some(22.0),
                    Some(24.0),
                ],
            )
            .unwrap(),
            Column::numeric(
                ColumnId(3),
                "First item",
                vec![
                    Some(8.0),
                    Some(10.0),
                    Some(12.0),
                    Some(9.0),
                    Some(11.0),
                    Some(15.0),
                ],
            )
            .unwrap(),
            Column::numeric(
                ColumnId(4),
                "Second item",
                vec![
                    Some(9.0),
                    Some(12.0),
                    Some(13.0),
                    Some(11.0),
                    Some(14.0),
                    Some(16.0),
                ],
            )
            .unwrap(),
            Column::text(
                ColumnId(5),
                "Group",
                [
                    "Control",
                    "Control",
                    "Control",
                    "Comparison",
                    "Comparison",
                    "Comparison",
                ]
                .into_iter()
                .map(|s| Some(s.into()))
                .collect(),
            )
            .unwrap(),
            Column::numeric(
                ColumnId(6),
                "Total score",
                vec![
                    Some(17.0),
                    Some(22.0),
                    Some(25.0),
                    Some(20.0),
                    Some(25.0),
                    Some(31.0),
                ],
            )
            .unwrap(),
        ]
        .into_iter()
        .map(|column| {
            let metadata = ColumnMetadata {
                measurement: match column.kind() {
                    ColumnKind::Number => Measurement::Scale,
                    ColumnKind::Text => Measurement::Nominal,
                },
                display_decimals: 0,
                description: if column.id() == ColumnId(6) {
                    "Sum of the two scale items".into()
                } else {
                    String::new()
                },
                ..column.metadata().clone()
            };
            column.with_metadata(metadata).unwrap()
        })
        .collect(),
    )
    .unwrap()
}
