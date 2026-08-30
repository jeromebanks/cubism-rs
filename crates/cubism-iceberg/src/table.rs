use std::collections::HashMap;

use arrow_schema::Schema as ArrowSchema;
use iceberg::spec::TableMetadata;
use iceberg::table::Table;
use iceberg::{Catalog, NamespaceIdent, TableCreation, TableIdent};

use crate::error::Result;
use crate::schema::{
    states_iceberg_schema, states_partition_spec, states_sort_order, xunit_registry_iceberg_schema,
};

/// The two Iceberg tables backing one cube's temporal aggregates: the
/// day-partitioned `states` table (Phase 2's `temporal_state_schema()`
/// shape) and the unpartitioned `xunit_registry` dictionary table. Matches
/// Phase 0B's selected `xunit_registry` physical layout — two tables, not
/// one wide/self-contained row.
pub struct TemporalTable {
    pub cube_id: String,
    pub states_ident: TableIdent,
    pub registry_ident: TableIdent,
}

impl TemporalTable {
    /// Create both tables under namespace `cube_id`, creating the namespace
    /// first if it does not already exist. `states_arrow_schema` should be
    /// `cubism_datafusion::temporal_build::temporal_state_schema(&spec)` —
    /// callers pass the Arrow schema across the crate boundary, never a
    /// `DataFrame`/`SessionContext` (see the crate's top-level doc comment).
    pub async fn create(
        catalog: &dyn Catalog,
        cube_id: &str,
        states_arrow_schema: &ArrowSchema,
    ) -> Result<Self> {
        let namespace = NamespaceIdent::new(cube_id.to_string());
        if !catalog.namespace_exists(&namespace).await? {
            catalog.create_namespace(&namespace, HashMap::new()).await?;
        }

        let states_name = format!("{cube_id}_states");
        let registry_name = format!("{cube_id}_xunit_registry");

        let states_ident = TableIdent::new(namespace.clone(), states_name.clone());
        if !catalog.table_exists(&states_ident).await? {
            let states_schema = states_iceberg_schema(states_arrow_schema)?;
            let states_creation = TableCreation::builder()
                .name(states_name.clone())
                .schema(states_schema)
                .partition_spec(states_partition_spec()?)
                .sort_order(states_sort_order()?)
                .build();
            catalog.create_table(&namespace, states_creation).await?;
        }

        let registry_ident = TableIdent::new(namespace.clone(), registry_name.clone());
        if !catalog.table_exists(&registry_ident).await? {
            let registry_schema = xunit_registry_iceberg_schema()?;
            let registry_creation = TableCreation::builder()
                .name(registry_name.clone())
                .schema(registry_schema)
                .build();
            catalog.create_table(&namespace, registry_creation).await?;
        }

        Ok(Self {
            cube_id: cube_id.to_string(),
            states_ident: TableIdent::new(namespace.clone(), states_name),
            registry_ident: TableIdent::new(namespace, registry_name),
        })
    }

    pub async fn states_table(&self, catalog: &dyn Catalog) -> Result<Table> {
        Ok(catalog.load_table(&self.states_ident).await?)
    }

    pub async fn registry_table(&self, catalog: &dyn Catalog) -> Result<Table> {
        Ok(catalog.load_table(&self.registry_ident).await?)
    }
}

pub fn current_snapshot_id(metadata: &TableMetadata) -> Option<i64> {
    metadata
        .current_snapshot()
        .map(|snapshot| snapshot.snapshot_id())
}
