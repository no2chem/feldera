use super::{
    ApiPermission, AttachedConnector, ConnectorDescr, ConnectorId, DBError, Pipeline,
    PipelineDescr, PipelineId, PipelineRevision, PipelineRuntimeState, PipelineStatus,
    ProgramDescr, ProgramId, ProgramSchema, Revision, Version,
};
use crate::api::ProgramStatus;
use crate::auth::TenantId;
use async_trait::async_trait;
use dbsp_adapters::{ConnectorConfig, RuntimeConfig};
use uuid::Uuid;

/// The storage trait contains the methods to interact with the pipeline manager
/// storage layer (e.g., PostgresDB) to implement the public API.
///
/// We use a trait so we can mock the storage layer in tests.
#[async_trait]
pub(crate) trait Storage {
    async fn list_programs(
        &self,
        tenant_id: TenantId,
        with_code: bool,
    ) -> Result<Vec<ProgramDescr>, DBError>;

    /// Retrieve program descriptor.
    ///
    /// Returns a `DBError:UnknownProgram` error if `program_id` is not found in
    /// the database.
    async fn get_program_by_id(
        &self,
        tenant_id: TenantId,
        program_id: ProgramId,
        with_code: bool,
    ) -> Result<ProgramDescr, DBError> {
        self.get_program_if_exists(tenant_id, program_id, with_code)
            .await?
            .ok_or(DBError::UnknownProgram { program_id })
    }

    /// Retrieve program descriptor.
    ///
    /// Returns a `DBError:UnknownName` error if `name` is not found in
    /// the database.
    async fn get_program_by_name(
        &self,
        tenant_id: TenantId,
        name: &str,
        with_code: bool,
    ) -> Result<ProgramDescr, DBError> {
        self.lookup_program(tenant_id, name, with_code)
            .await?
            .ok_or_else(|| DBError::UnknownName { name: name.into() })
    }

    /// Validate program version and retrieve program descriptor.
    ///
    /// Returns `DBError::UnknownProgram` if `program_id` is not found in the
    /// database. Returns `DBError::OutdatedProgramVersion` if the current
    /// program version differs from `expected_version`.
    async fn get_program_guarded(
        &self,
        tenant_id: TenantId,
        program_id: ProgramId,
        expected_version: Version,
    ) -> Result<ProgramDescr, DBError> {
        let descr = self.get_program_by_id(tenant_id, program_id, false).await?;
        if descr.version != expected_version {
            return Err(DBError::OutdatedProgramVersion {
                latest_version: descr.version,
            });
        }

        Ok(descr)
    }

    /// Queue program for compilation by setting its status to
    /// [`ProgramStatus::Pending`] and schema to null.
    ///
    /// Change program status to [`ProgramStatus::Pending`].
    async fn prepare_program_for_compilation(
        &self,
        tenant_id: TenantId,
        program_id: ProgramId,
        expected_version: Version,
    ) -> Result<(), DBError> {
        let descr = self
            .get_program_guarded(tenant_id, program_id, expected_version)
            .await?;

        // Do nothing if the program:
        // * is already pending (we don't want to bump its `status_since` field, which would move
        //   it to the end of the queue),
        // * if compilation is already in progress,
        // * or if the program has already been compiled
        if descr.status == ProgramStatus::Pending
            || descr.status.is_compiling()
            || descr.status == ProgramStatus::Success
        {
            return Ok(());
        }

        self.set_program_for_compilation(
            tenant_id,
            program_id,
            expected_version,
            ProgramStatus::Pending,
        )
        .await?;

        Ok(())
    }

    /// Create a new program.
    async fn new_program(
        &self,
        tenant_id: TenantId,
        id: Uuid,
        program_name: &str,
        program_description: &str,
        program_code: &str,
    ) -> Result<(ProgramId, Version), DBError>;

    /// Update program name, description and, optionally, code.
    /// XXX: Description should be optional too
    async fn update_program(
        &self,
        tenant_id: TenantId,
        program_id: ProgramId,
        program_name: &str,
        program_description: &str,
        program_code: &Option<String>,
    ) -> Result<Version, DBError>;

    /// Retrieve program descriptor.
    ///
    /// Returns `None` if `program_id` is not found in the database.
    async fn get_program_if_exists(
        &self,
        tenant_id: TenantId,
        program_id: ProgramId,
        with_code: bool,
    ) -> Result<Option<ProgramDescr>, DBError>;

    /// Lookup program by name.
    async fn lookup_program(
        &self,
        tenant_id: TenantId,
        program_name: &str,
        with_code: bool,
    ) -> Result<Option<ProgramDescr>, DBError>;

    /// Update program status.
    ///
    /// # Note
    /// - Doesn't check that the program exists.
    /// - Resets schema to null.
    async fn set_program_for_compilation(
        &self,
        tenant_id: TenantId,
        program_id: ProgramId,
        version: Version,
        status: ProgramStatus,
    ) -> Result<(), DBError>;

    /// Update program status after a version check.
    ///
    /// Updates program status to `status` if the current program version in the
    /// database matches `expected_version`.
    ///
    /// # Note
    /// This intentionally does not throw an error if there is a program version
    /// mismatch and instead does just not update. It's used by the compiler to
    /// update status and in case there is a newer version it is expected that
    /// the compiler just picks up and runs the next job.
    async fn set_program_status_guarded(
        &self,
        tenant_id: TenantId,
        program_id: ProgramId,
        expected_version: Version,
        status: ProgramStatus,
    ) -> Result<(), DBError>;

    /// Update program schema.
    ///
    /// # Note
    /// This should be called after the SQL compilation succeeded, e.g., in the
    /// same transaction that sets status to  [`ProgramStatus::CompilingRust`].
    async fn set_program_schema(
        &self,
        tenant_id: TenantId,
        program_id: ProgramId,
        schema: ProgramSchema,
    ) -> Result<(), DBError>;

    /// Delete program from the database.
    ///
    /// This will delete all program configs and pipelines.
    async fn delete_program(
        &self,
        tenant_id: TenantId,
        program_id: ProgramId,
    ) -> Result<(), DBError>;

    /// Retrieves all programs in the DB. Intended to be used by
    /// reconciliation loops.
    async fn all_programs(&self) -> Result<Vec<(TenantId, ProgramDescr)>, DBError>;

    /// Retrieves all pipelines in the DB. Intended to be used by
    /// reconciliation loops.
    async fn all_pipelines(&self) -> Result<Vec<(TenantId, PipelineId)>, DBError>;

    /// Retrieves the first pending program from the queue.
    ///
    /// Returns a pending program with the most recent `status_since` or `None`
    /// if there are no pending programs in the DB.
    async fn next_job(&self) -> Result<Option<(TenantId, ProgramId, Version)>, DBError>;

    /// Version the configuration for a pipeline.
    ///
    /// Returns the revision number for that snapshot.
    async fn create_pipeline_revision(
        &self,
        new_revision_id: Uuid,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
    ) -> Result<Revision, DBError>;

    /// Retrieves the current revision for a pipeline (including all immutable
    /// state needed to run it).
    async fn get_last_committed_pipeline_revision(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
    ) -> Result<PipelineRevision, DBError>;

    /// Create a new config.
    #[allow(clippy::too_many_arguments)]
    async fn new_pipeline(
        &self,
        tenant_id: TenantId,
        id: Uuid,
        program_id: Option<ProgramId>,
        pipline_name: &str,
        pipeline_description: &str,
        config: &RuntimeConfig,
        connectors: &Option<Vec<AttachedConnector>>,
    ) -> Result<(PipelineId, Version), DBError>;

    /// Update existing config.
    ///
    /// Update config name and, optionally, YAML.
    #[allow(clippy::too_many_arguments)]
    async fn update_pipeline(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
        program_id: Option<ProgramId>,
        pipline_name: &str,
        pipeline_description: &str,
        config: &Option<RuntimeConfig>,
        connectors: &Option<Vec<AttachedConnector>>,
    ) -> Result<Version, DBError>;

    /// Delete config.
    async fn delete_config(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
    ) -> Result<(), DBError>;

    /// Get input/output status for an attached connector.
    async fn attached_connector_is_input(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
        name: &str,
    ) -> Result<bool, DBError>;

    /// Delete `pipeline` from the DB.
    async fn delete_pipeline(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
    ) -> Result<bool, DBError>;

    /// Retrieve pipeline for a given id.
    async fn get_pipeline_descr_by_id(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
    ) -> Result<PipelineDescr, DBError>;

    async fn get_pipeline_by_id(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
    ) -> Result<Pipeline, DBError>;

    /// Retrieve pipeline for a given name.
    async fn get_pipeline_descr_by_name(
        &self,
        tenant_id: TenantId,
        name: String,
    ) -> Result<PipelineDescr, DBError>;

    async fn get_pipeline_by_name(
        &self,
        tenant_id: TenantId,
        name: String,
    ) -> Result<Pipeline, DBError>;

    async fn get_pipeline_runtime_state(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
    ) -> Result<PipelineRuntimeState, DBError>;

    async fn update_pipeline_runtime_state(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
        state: &PipelineRuntimeState,
    ) -> Result<(), DBError>;

    async fn set_pipeline_desired_status(
        &self,
        tenant_id: TenantId,
        pipeline_id: PipelineId,
        desired_status: PipelineStatus,
    ) -> Result<(), DBError>;

    async fn list_pipelines(&self, tenant_id: TenantId) -> Result<Vec<Pipeline>, DBError>;

    /// Create a new connector.
    async fn new_connector(
        &self,
        tenant_id: TenantId,
        id: Uuid,
        name: &str,
        description: &str,
        config: &ConnectorConfig,
    ) -> Result<ConnectorId, DBError>;

    /// Retrieve connectors list from the DB.
    async fn list_connectors(&self, tenant_id: TenantId) -> Result<Vec<ConnectorDescr>, DBError>;

    /// Retrieve connector descriptor for the given `connector_id`.
    async fn get_connector_by_id(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
    ) -> Result<ConnectorDescr, DBError>;

    /// Retrieve connector descriptor for the given `name`.
    async fn get_connector_by_name(
        &self,
        tenant_id: TenantId,
        name: String,
    ) -> Result<ConnectorDescr, DBError>;

    /// Update existing connector config.
    ///
    /// Update connector name and, optionally, YAML.
    async fn update_connector(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        connector_name: &str,
        description: &str,
        config: &Option<ConnectorConfig>,
    ) -> Result<(), DBError>;

    /// Delete connector from the database.
    ///
    /// This will delete all connector configs and pipelines.
    async fn delete_connector(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
    ) -> Result<(), DBError>;

    /// Persist a hash of API key in the database
    async fn store_api_key_hash(
        &self,
        tenant_id: TenantId,
        key: String,
        permissions: Vec<ApiPermission>,
    ) -> Result<(), DBError>;

    /// Validate an API key against the database
    async fn validate_api_key(
        &self,
        key: String,
    ) -> Result<(TenantId, Vec<ApiPermission>), DBError>;

    /// Get the tenant ID from the database for a given tenant name and
    /// provider, else create a new tenant ID
    async fn get_or_create_tenant_id(
        &self,
        tenant_name: String,
        provider: String,
    ) -> Result<TenantId, DBError>;

    /// Create a new tenant ID for a given tenant name and provider
    async fn create_tenant_if_not_exists(
        &self,
        tenant_id: Uuid,
        tenant_name: String,
        provider: String,
    ) -> Result<TenantId, DBError>;

    /// Record a URL pointing to a compile. binary Supported URL types are determined by the
    /// compiler service (e.g. file:///)
    async fn create_compiled_binary_ref(
        &self,
        program_id: ProgramId,
        version: Version,
        url: String,
    ) -> Result<(), DBError>;

    /// Retrieve a compiled binary's URL
    async fn get_compiled_binary_ref(
        &self,
        program_id: ProgramId,
        version: Version,
    ) -> Result<Option<String>, DBError>;

    /// Retrieve a compiled binary's URL
    async fn delete_compiled_binary_ref(
        &self,
        program_id: ProgramId,
        version: Version,
    ) -> Result<(), DBError>;
}
