use arrow_schema::{DataType, Schema as ArrowSchema, TimeUnit};
use iceberg::spec::{
    NestedField, NestedFieldRef, NullOrder, PrimitiveType, Schema as IcebergSchema, SortDirection,
    SortField, SortOrder, Transform, Type, UnboundPartitionSpec,
};

use crate::error::{CubismIcebergError, Result};

/// Identity columns prepended to Phase 2's own state schema. `cube_id` is
/// not one of them — it is implicit in the table name (`<cube_id>_states`),
/// one states table per cube. `window_id`/`revision`/`run_id` must be
/// per-row: one states table accumulates every build of every window over
/// time, and [`crate::reader::AggregateReader`] filters rows to the
/// publication store's current revision for a window by reading this
/// `revision` column directly (no join engine — see the reader's doc
/// comment).
pub const WINDOW_ID_FIELD_ID: i32 = 1;
pub const REVISION_FIELD_ID: i32 = 2;
pub const RUN_ID_FIELD_ID: i32 = 3;
const IDENTITY_FIELD_COUNT: i32 = 3;

pub const WINDOW_ID_COLUMN: &str = "window_id";
pub const REVISION_COLUMN: &str = "revision";
pub const RUN_ID_COLUMN: &str = "run_id";

/// Convert a Cubism temporal-build Arrow schema (`bucket_start, xunit_id,
/// <measure>_v<N>...`, see `cubism_datafusion::temporal_build::temporal_state_schema`)
/// into the Iceberg schema for the states table, prefixed with the identity
/// columns above.
///
/// Field IDs are assigned explicitly (identity columns 1-3, then the Arrow
/// schema's own columns in order starting at 4) — a declared authority, not
/// something inferred from Arrow metadata, matching the plan's "Store field
/// IDs and canonical schema fixtures" requirement.
pub fn states_iceberg_schema(arrow_schema: &ArrowSchema) -> Result<IcebergSchema> {
    let mut fields = vec![
        NestedFieldRef::new(NestedField::required(
            WINDOW_ID_FIELD_ID,
            WINDOW_ID_COLUMN,
            Type::Primitive(PrimitiveType::String),
        )),
        NestedFieldRef::new(NestedField::required(
            REVISION_FIELD_ID,
            REVISION_COLUMN,
            Type::Primitive(PrimitiveType::Long),
        )),
        NestedFieldRef::new(NestedField::required(
            RUN_ID_FIELD_ID,
            RUN_ID_COLUMN,
            Type::Primitive(PrimitiveType::String),
        )),
    ];
    for (index, field) in arrow_schema.fields().iter().enumerate() {
        let id = IDENTITY_FIELD_COUNT + (index as i32) + 1;
        let primitive = arrow_type_to_iceberg_primitive(field.name(), field.data_type())?;
        let nested = if field.is_nullable() {
            NestedField::optional(id, field.name(), Type::Primitive(primitive))
        } else {
            NestedField::required(id, field.name(), Type::Primitive(primitive))
        };
        fields.push(NestedFieldRef::new(nested));
    }
    IcebergSchema::builder()
        .with_schema_id(0)
        .with_fields(fields)
        .build()
        .map_err(CubismIcebergError::Iceberg)
}

/// The Iceberg field ID `bucket_start` gets once prefixed by the identity
/// columns in [`states_iceberg_schema`] (Arrow column 0 -> field 4).
pub fn bucket_start_field_id() -> i32 {
    IDENTITY_FIELD_COUNT + 1
}

/// The Iceberg field ID `xunit_id` gets once prefixed by the identity
/// columns in [`states_iceberg_schema`] (Arrow column 1 -> field 5).
pub fn xunit_id_field_id() -> i32 {
    IDENTITY_FIELD_COUNT + 2
}

/// `(xunit_id: FixedSizeBinary(32), xunit_canonical: Binary)` — fixed,
/// matches `temporal_build::build_temporal`'s registry `DataFrame` shape
/// exactly (not derived from a spec, unlike the states schema). No identity
/// columns: the registry is a content-addressed dictionary (same `xunit_id`
/// always maps to the same canonical bytes), so rows from different runs
/// can be appended without provenance and simply accumulate.
pub fn xunit_registry_iceberg_schema() -> Result<IcebergSchema> {
    IcebergSchema::builder()
        .with_schema_id(0)
        .with_fields(vec![
            NestedFieldRef::new(NestedField::required(
                1,
                "xunit_id",
                Type::Primitive(PrimitiveType::Fixed(32)),
            )),
            NestedFieldRef::new(NestedField::required(
                2,
                "xunit_canonical",
                Type::Primitive(PrimitiveType::Binary),
            )),
        ])
        .build()
        .map_err(CubismIcebergError::Iceberg)
}

fn arrow_type_to_iceberg_primitive(field_name: &str, data_type: &DataType) -> Result<PrimitiveType> {
    match data_type {
        DataType::Timestamp(TimeUnit::Microsecond, Some(tz)) if tz.as_ref() == "+00:00" => {
            Ok(PrimitiveType::Timestamptz)
        }
        DataType::FixedSizeBinary(32) => Ok(PrimitiveType::Fixed(32)),
        DataType::Binary | DataType::LargeBinary => Ok(PrimitiveType::Binary),
        DataType::Float64 => Ok(PrimitiveType::Double),
        DataType::Int64 => Ok(PrimitiveType::Long),
        other => Err(CubismIcebergError::Iceberg(iceberg::Error::new(
            iceberg::ErrorKind::FeatureUnsupported,
            format!("column '{field_name}' has unsupported Arrow type {other:?} for an Iceberg states table"),
        ))),
    }
}

/// Day-partition the states table on `bucket_start` — matches Phase 0B's
/// selected `xunit_registry` layout and the Phase 0A spike's proven
/// day-partition pruning. The `xunit_registry` table itself is
/// unpartitioned (no time column).
pub fn states_partition_spec() -> Result<UnboundPartitionSpec> {
    Ok(UnboundPartitionSpec::builder()
        .with_spec_id(0)
        .add_partition_field(bucket_start_field_id(), "bucket_day", Transform::Day)
        .map_err(CubismIcebergError::Iceberg)?
        .build())
}

/// `(bucket_start, xunit_id)` ascending — matches Phase 2's own output
/// order (`temporal_build`'s end-to-end test asserts this ordering across
/// the whole collected batch stream).
pub fn states_sort_order() -> Result<SortOrder> {
    let field = |source_id: i32| {
        SortField::builder()
            .source_id(source_id)
            .transform(Transform::Identity)
            .direction(SortDirection::Ascending)
            .null_order(NullOrder::First)
            .build()
    };
    SortOrder::builder()
        .with_order_id(1)
        .with_sort_field(field(bucket_start_field_id()))
        .with_sort_field(field(xunit_id_field_id()))
        .build_unbound()
        .map_err(CubismIcebergError::Iceberg)
}
