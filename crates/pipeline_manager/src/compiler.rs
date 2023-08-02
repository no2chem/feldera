use crate::auth::TenantId;
use crate::config::CompilerConfig;
use crate::db::storage::Storage;
use crate::db::{DBError, ProgramId, ProjectDB, Version};
use crate::error::ManagerError;
use log::warn;
use log::{debug, error, info, trace};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::{
    process::{ExitStatus, Stdio},
    sync::Arc,
};
use tokio::fs::DirEntry;
use tokio::io::AsyncWriteExt;
use tokio::{
    fs,
    fs::{File, OpenOptions},
    process::{Child, Command},
    select, spawn,
    sync::Mutex,
    task::JoinHandle,
    time::{sleep, Duration},
};
use utoipa::ToSchema;
use uuid::Uuid;

/// The frequency with which the compiler polls the project database
/// for new compilation requests.
const COMPILER_POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// The frequency with which the GC task checks for no longer used binaries
/// against the database and removes the orphans.
///
/// # TODO
/// Artifcially low limit at the moment since this is new code. Increase in the
/// future.
const GC_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// A SQL compiler error.
///
/// The SQL compiler returns a list of errors in the following JSON format if
/// it's invoked with the `-je` option.
///
/// ```ignore
///  [ {
/// "startLineNumber" : 14,
/// "startColumn" : 13,
/// "endLineNumber" : 14,
/// "endColumn" : 13,
/// "warning" : false,
/// "errorType" : "Error parsing SQL",
/// "message" : "Encountered \"<EOF>\" at line 14, column 13."
/// } ]
/// ```
#[derive(Debug, Deserialize, Serialize, Eq, PartialEq, ToSchema, Clone)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub(crate) struct SqlCompilerMessage {
    start_line_number: usize,
    start_column: usize,
    end_line_number: usize,
    end_column: usize,
    warning: bool,
    error_type: String,
    message: String,
}

/// Program compilation status.
#[derive(Debug, Deserialize, Serialize, Eq, PartialEq, ToSchema, Clone)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(crate) enum ProgramStatus {
    /// Initial state: program has been created or modified, but the user
    /// hasn't yet started compiling the program.
    None,
    /// Compilation request received from the user; program has been placed
    /// in the queue.
    Pending,
    /// Compilation of SQL -> Rust in progress.
    CompilingSql,
    /// Compiling Rust -> executable in progress
    CompilingRust,
    /// Compilation succeeded.
    #[cfg_attr(test, proptest(weight = 2))]
    Success,
    /// SQL compiler returned an error.
    SqlError(Vec<SqlCompilerMessage>),
    /// Rust compiler returned an error.
    RustError(String),
    /// System/OS returned an error when trying to invoke commands.
    SystemError(String),
}

impl ProgramStatus {
    /// Return true if program is not yet compiled but might be in the future.
    pub(crate) fn is_not_yet_compiled(&self) -> bool {
        *self == ProgramStatus::None || *self == ProgramStatus::Pending
    }

    /// Return true if the program has failed to compile (for any reason).
    pub(crate) fn has_failed_to_compile(&self) -> bool {
        matches!(
            self,
            ProgramStatus::SqlError(_)
                | ProgramStatus::RustError(_)
                | ProgramStatus::SystemError(_)
        )
    }

    /// Return true if program is currently compiling.
    pub(crate) fn is_compiling(&self) -> bool {
        *self == ProgramStatus::CompilingRust || *self == ProgramStatus::CompilingSql
    }
}

pub struct Compiler {
    pub compiler_task: JoinHandle<Result<(), ManagerError>>,
    gc_task: JoinHandle<Result<(), ManagerError>>,
}

impl Drop for Compiler {
    fn drop(&mut self) {
        self.compiler_task.abort();
        self.gc_task.abort();
    }
}

/// The `main` function injected in each generated pipeline
/// crate.
const MAIN_FUNCTION: &str = r#"
fn main() {
    dbsp_adapters::server::server_main(&circuit).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
}"#;

impl Compiler {
    pub async fn new(
        config: &CompilerConfig,
        db: Arc<Mutex<ProjectDB>>,
    ) -> Result<Self, ManagerError> {
        Self::create_working_directory(config).await?;
        let compiler_task = spawn(Self::compiler_task(config.clone(), db.clone()));
        let gc_task = spawn(Self::gc_task(config.clone(), db));
        Ok(Self {
            compiler_task,
            gc_task,
        })
    }

    async fn create_working_directory(config: &CompilerConfig) -> Result<(), ManagerError> {
        fs::create_dir_all(&config.workspace_dir())
            .await
            .map_err(|e| {
                ManagerError::io_error(
                    format!(
                        "creating Rust workspace directory '{}'",
                        config.workspace_dir().display()
                    ),
                    e,
                )
            })
    }

    /// Download and compile all crates needed by the Rust code generated
    /// by the SQL compiler.
    ///
    /// This is useful to prepare the working directory, so that the first
    /// compilation job completes quickly.  Also creates the `Cargo.lock`
    /// file, making sure that subsequent `cargo` runs do not access the
    /// network.
    pub async fn precompile_dependencies(config: &CompilerConfig) -> Result<(), ManagerError> {
        let program_id = ProgramId(Uuid::nil());

        Self::create_working_directory(config).await?;

        // Create workspace-level Cargo.toml
        Compiler::write_workspace_toml(config, program_id).await?;

        // Create dummy package.
        let rust_file_path = config.rust_program_path(program_id);
        let rust_src_dir = rust_file_path.parent().unwrap();
        fs::create_dir_all(rust_src_dir).await.map_err(|e| {
            ManagerError::io_error(
                format!(
                    "creating Rust source directory '{}'",
                    rust_src_dir.display()
                ),
                e,
            )
        })?;

        Compiler::write_project_toml(config, program_id).await?;

        fs::write(&rust_file_path, "fn main() {}")
            .await
            .map_err(|e| {
                ManagerError::io_error(format!("writing '{}'", rust_file_path.display()), e)
            })?;

        // `cargo build`.
        let mut cargo_process = Compiler::run_cargo_build(config, program_id).await?;
        let exit_status = cargo_process
            .wait()
            .await
            .map_err(|e| ManagerError::io_error("waiting for 'cargo build'".to_string(), e))?;

        if !exit_status.success() {
            let stdout = fs::read_to_string(config.compiler_stdout_path(program_id))
                .await
                .map_err(|e| {
                    ManagerError::io_error(
                        format!(
                            "reading '{}'",
                            config.compiler_stdout_path(program_id).display()
                        ),
                        e,
                    )
                })?;
            let stderr = fs::read_to_string(config.compiler_stderr_path(program_id))
                .await
                .map_err(|e| {
                    ManagerError::io_error(
                        format!(
                            "reading '{}'",
                            config.compiler_stderr_path(program_id).display()
                        ),
                        e,
                    )
                })?;
            return Err(ManagerError::RustCompilerError {
                error: format!(
                    "Failed to precompile Rust dependencies\nstdout:\n{stdout}\nstderr:\n{stderr}"
                ),
            });
        }

        Ok(())
    }

    async fn run_cargo_build(
        config: &CompilerConfig,
        program_id: ProgramId,
    ) -> Result<Child, ManagerError> {
        let err_file = File::create(&config.compiler_stderr_path(program_id))
            .await
            .map_err(|e| {
                ManagerError::io_error(
                    format!(
                        "creating '{}'",
                        config.compiler_stderr_path(program_id).display()
                    ),
                    e,
                )
            })?;
        let out_file = File::create(&config.compiler_stdout_path(program_id))
            .await
            .map_err(|e| {
                ManagerError::io_error(
                    format!(
                        "creating '{}'",
                        config.compiler_stdout_path(program_id).display()
                    ),
                    e,
                )
            })?;

        let mut command = Command::new("cargo");

        command
            .current_dir(&config.workspace_dir())
            .arg("build")
            .arg("--workspace")
            .stdin(Stdio::null())
            .stderr(Stdio::from(err_file.into_std().await))
            .stdout(Stdio::from(out_file.into_std().await));

        if !config.debug {
            command.arg("--release");
        }

        command
            .spawn()
            .map_err(|e| ManagerError::io_error("starting 'cargo'".to_string(), e))
    }

    async fn version_binary(
        config: &CompilerConfig,
        program_id: ProgramId,
        version: Version,
    ) -> Result<(), ManagerError> {
        info!(
            "Preserve binary {:?} as {:?}",
            config.target_executable(program_id),
            config.versioned_executable(program_id, version)
        );
        let source = config.target_executable(program_id);
        let destination = config.versioned_executable(program_id, version);
        fs::copy(&source, &destination).await.map_err(|e| {
            ManagerError::io_error(
                format!(
                    "copying '{}' to '{}'",
                    source.display(),
                    destination.display()
                ),
                e,
            )
        })?;
        Ok(())
    }

    /// Generate workspace-level `Cargo.toml`.
    async fn write_workspace_toml(
        config: &CompilerConfig,
        program_id: ProgramId,
    ) -> Result<(), ManagerError> {
        let workspace_toml_code = format!(
            "[workspace]\nmembers = [ \"{}\" ]\n",
            CompilerConfig::crate_name(program_id),
        );
        let toml_path = config.workspace_toml_path();
        fs::write(&toml_path, workspace_toml_code)
            .await
            .map_err(|e| ManagerError::io_error(format!("writing '{}'", toml_path.display()), e))?;

        Ok(())
    }

    /// Generate project-level `Cargo.toml`.
    async fn write_project_toml(
        config: &CompilerConfig,
        program_id: ProgramId,
    ) -> Result<(), ManagerError> {
        let template_path = config.project_toml_template_path();
        let template_toml = fs::read_to_string(&template_path).await.map_err(|e| {
            ManagerError::io_error(format!("reading template '{}'", template_path.display()), e)
        })?;
        let program_name = format!("name = \"{}\"", CompilerConfig::crate_name(program_id));
        let mut project_toml_code = template_toml
            .replace("name = \"temp\"", &program_name)
            .replace(", default-features = false", "")
            .replace(
                "[lib]\npath = \"src/lib.rs\"",
                &format!("\n\n[[bin]]\n{program_name}\npath = \"src/main.rs\""),
            );
        if let Some(p) = &config.dbsp_override_path {
            project_toml_code = project_toml_code
                .replace("../../crates", &format!("{p}/crates"))
                .replace("../lib", &format!("{}", config.sql_lib_path().display()));
        };
        debug!("TOML:\n{project_toml_code}");

        let toml_path = config.project_toml_path(program_id);
        fs::write(&toml_path, project_toml_code)
            .await
            .map_err(|e| ManagerError::io_error(format!("writing '{}'", toml_path.display()), e))?;

        Ok(())
    }

    async fn gc_task(
        config: CompilerConfig,
        db: Arc<Mutex<ProjectDB>>,
    ) -> Result<(), ManagerError> {
        Self::do_gc_task(config, db).await.map_err(|e| {
            error!("gc task failed; error: '{e}'");
            e
        })
    }

    async fn binary_path_to_parts(path: &DirEntry) -> Option<(ProgramId, Version)> {
        let file_name = path.file_name();
        let file_name = file_name.to_str().unwrap();
        if file_name.starts_with("project") {
            let parts: Vec<&str> = file_name.split('_').collect();
            if parts.len() != 3
                || parts.first() != Some(&"project")
                || parts.get(2).map(|p| p.len()) <= Some(1)
            {
                error!("Invalid binary file found: {}", file_name);
                return None;
            }
            if let (Ok(program_uuid), Ok(program_version)) =
                // parse file name with the following format:
                // project_{uuid}_v{version}
                (Uuid::parse_str(parts[1]), parts[2][1..].parse::<i64>())
            {
                return Some((ProgramId(program_uuid), Version(program_version)));
            }
        }
        None
    }

    /// A task that wakes up periodically and removes stale binaries.
    ///
    /// Helps to keep the binaries directory clean and not run out of space if
    /// this runs for a very long time.
    ///
    /// Note that this task handles all errors internally and does not propagate
    /// them up so it can run forever and never aborts.
    async fn do_gc_task(
        config: CompilerConfig,
        db: Arc<Mutex<ProjectDB>>,
    ) -> Result<(), ManagerError> {
        loop {
            sleep(GC_POLL_INTERVAL).await;
            let read_dir = fs::read_dir(config.binaries_dir()).await;
            match read_dir {
                Ok(mut paths) => loop {
                    let entry = paths.next_entry().await;
                    match entry {
                        Ok(Some(path)) => {
                            let maybe_parts = Self::binary_path_to_parts(&path).await;
                            match maybe_parts {
                                Some((program_uuid, program_version)) => {
                                    if let Ok(is_used) = db
                                        .lock()
                                        .await
                                        .is_program_version_in_use(
                                            program_uuid.0,
                                            program_version.0,
                                        )
                                        .await
                                    {
                                        if !is_used {
                                            warn!("About to remove binary file '{:?}' that is no longer in use by any program", path.file_name());
                                            let r = fs::remove_file(path.path()).await;
                                            if let Err(e) = r {
                                                error!(
                                                    "GC task failed to remove file '{:?}': {}",
                                                    path.file_name(),
                                                    e
                                                );
                                            }
                                        }
                                    }
                                }
                                None => {
                                    warn!(
                                        "GC task found invalid file in {:?}: {:?}",
                                        config.binaries_dir(),
                                        path.file_name()
                                    );
                                }
                            }
                        }
                        // We are done with the directory
                        Ok(None) => break,
                        Err(e) => {
                            warn!("GC task unable to read an entry: {}", e);
                            // Not clear from docs if an error during iteration
                            // is recoverable and we could just `continue;` here
                            break;
                        }
                    }
                },
                Err(e) => {
                    error!("GC task couldn't read binaries directory: {}", e)
                }
            }
        }
    }

    async fn compiler_task(
        config: CompilerConfig,
        db: Arc<Mutex<ProjectDB>>,
    ) -> Result<(), ManagerError> {
        Self::do_compiler_task(config, db).await.map_err(|e| {
            error!("compiler task failed; error: '{e}'");
            e
        })
    }

    /// Invoked at startup so the compiler service can align its
    /// local state (versioned executables) with the Programs API state.
    /// It recovers from partially progressed compilation jobs.
    async fn reconcile_local_state(
        config: &CompilerConfig,
        db: &Arc<Mutex<ProjectDB>>,
    ) -> Result<(), DBError> {
        info!("Reconciling local state with API state");
        let mut map: HashSet<(Uuid, i64)> = HashSet::new();
        let read_dir = fs::read_dir(config.binaries_dir()).await;
        match read_dir {
            Ok(mut paths) => loop {
                let entry = paths.next_entry().await;
                match entry {
                    Ok(Some(path)) => {
                        let maybe_parts = Self::binary_path_to_parts(&path).await;
                        match maybe_parts {
                            Some((program_uuid, program_version)) => {
                                map.insert((program_uuid.0, program_version.0));
                            }
                            None => {
                                warn!(
                                    "Local state reconciler found invalid file in {:?}: {:?}",
                                    config.binaries_dir(),
                                    path.file_name()
                                );
                            }
                        }
                    }
                    // We are done with the directory
                    Ok(None) => break,
                    Err(e) => {
                        warn!("Local state reconciler was unable to read an entry: {}", e);
                        // Not clear from docs if an error during iteration
                        // is recoverable and we could just `continue;` here
                        break;
                    }
                }
            },
            Err(e) => {
                warn!(
                    "Local state reconciler could not read binaries directory: {}",
                    e
                )
            }
        }

        let programs = db.lock().await.all_programs().await?;
        for (tenant_id, program) in programs {
            // We have some artifact but the program status has not been updated to Success.
            // This could indicate a failure between when we began writing the versioned executable
            // to before we could update the program status. This means, the best solution
            // is to start over with the compilation
            if program.status == ProgramStatus::CompilingRust
                && map.contains(&(program.program_id.0, program.version.0))
            {
                let path = config.versioned_executable(program.program_id, program.version);
                info!("File {:?} exists, but the program status is CompilingRust. Removing the file to start compilaiton again.", path.file_name());
                let r = fs::remove_file(path.clone()).await;
                if let Err(e) = r {
                    error!(
                        "Reconcile task failed to remove file '{:?}': {}",
                        path.file_name(),
                        e
                    );
                }
                db.lock()
                    .await
                    .set_program_status_guarded(
                        tenant_id,
                        program.program_id,
                        program.version,
                        ProgramStatus::Pending,
                    )
                    .await?;
            }
            // If the program was supposed to be further in the compilation chain, but
            // we don't have the binary artifact available locally, we need to queue the program
            // for compilation again. TODO: this behavior will change when the compiler uploads
            // artifacts remotely and not on its local filesystem.
            else if (program.status.is_compiling() || program.status == ProgramStatus::Success)
                && !map.contains(&(program.program_id.0, program.version.0))
            {
                info!(
                    "Program {} does not have a local artifact despite being in the {:?} state. Resetting compilation status.",
                    program.program_id, program.status
                );
                db.lock()
                    .await
                    .set_program_for_compilation(
                        tenant_id,
                        program.program_id,
                        program.version,
                        ProgramStatus::Pending,
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn do_compiler_task(
        /* command_receiver: Receiver<CompilerCommand>, */
        config: CompilerConfig,
        db: Arc<Mutex<ProjectDB>>,
    ) -> Result<(), ManagerError> {
        let mut job: Option<CompilationJob> = None;
        Self::reconcile_local_state(&config, &db).await?;
        loop {
            select! {
                // Wake up every `COMPILER_POLL_INTERVAL` to check
                // if we need to abort ongoing compilation.
                _ = sleep(COMPILER_POLL_INTERVAL) => {
                    let mut cancel = false;
                    if let Some(job) = &job {
                        // Program was deleted, updated or the user changed its status
                        // to cancelled -- abort compilation.
                        let descr = db.lock().await.get_program_if_exists(job.tenant_id, job.program_id, false).await?;
                        if let Some(descr) = descr {
                            if descr.version != job.version || !descr.status.is_compiling() {
                                cancel = true;
                            }
                        } else {
                            cancel = true;
                        }
                    }
                    if cancel {
                        job.unwrap().cancel().await;
                        job = None;
                    }
                }
                // Compilation job finished - start the next stage of the compilation
                // (i.e. run the Rust compiler after SQL) or update program status in the
                // database.
                Some(exit_status) = async {
                    if let Some(job) = &mut job {
                        Some(job.wait().await)
                    } else {
                        None
                    }
                }, if job.is_some() => {
                    let tenant_id = job.as_ref().unwrap().tenant_id;
                    let program_id = job.as_ref().unwrap().program_id;
                    let version = job.as_ref().unwrap().version;
                    let db = db.lock().await;

                    match exit_status {
                        Ok(status) if status.success() && job.as_ref().unwrap().is_sql() => {
                            // SQL compiler succeeded -- start the Rust job.
                            db.set_program_status_guarded(
                                tenant_id,
                                program_id,
                                version,
                                ProgramStatus::CompilingRust,
                            ).await?;

                            // Read the schema so we can store it in the DB.
                            //
                            // - We trust the compiler that it put the file
                            // there if it succeeded.
                            // - We hold the db lock so we are executing this
                            // update in the same transaction as the program
                            // status above.

                            let schema_path = config.schema_path(program_id);
                            let schema_json = fs::read_to_string(&schema_path).await
                                .map_err(|e| {
                                    ManagerError::io_error(format!("reading '{}'", schema_path.display()), e)
                                })?;

                            let schema = serde_json::from_str(&schema_json)
                                .map_err(|e| { ManagerError::invalid_program_schema(e.to_string()) })?;
                            db.set_program_schema(tenant_id, program_id, schema).await?;

                            debug!("Set ProgramStatus::CompilingRust '{program_id}', version '{version}'");
                            job = Some(CompilationJob::rust(tenant_id, &config, program_id, version).await?);
                        }
                        Ok(status) if status.success() && job.as_ref().unwrap().is_rust() => {
                            Self::version_binary(&config, program_id, version).await?;
                            // Rust compiler succeeded -- declare victory.
                            db.set_program_status_guarded(tenant_id, program_id, version, ProgramStatus::Success).await?;
                            debug!("Set ProgramStatus::Success '{program_id}', version '{version}'");
                            job = None;
                        }
                        Ok(status) => {
                            // Compilation failed - update program status with the compiler
                            // error message.
                            let output = job.as_ref().unwrap().error_output(&config).await?;
                            let status = if job.as_ref().unwrap().is_rust() {
                                ProgramStatus::RustError(format!("{output}\nexit code: {status}"))
                            } else if let Ok(messages) = serde_json::from_str(&output) {
                                    // If we can parse the SqlCompilerMessages
                                    // as JSON, we assume the compiler worked:
                                    ProgramStatus::SqlError(messages)
                            } else {
                                    // Otherwise something unexpected happened
                                    // and we return a system error:
                                    ProgramStatus::SystemError(format!("{output}\nexit code: {status}"))
                            };
                            db.set_program_status_guarded(tenant_id, program_id, version, status).await?;
                            job = None;
                        }
                        Err(e) => {
                            let status = if job.unwrap().is_rust() {
                                ProgramStatus::SystemError(format!("I/O error with rustc: {e}"))
                            } else {
                                ProgramStatus::SystemError(format!("I/O error with sql-to-dbsp: {e}"))
                            };
                            db.set_program_status_guarded(tenant_id, program_id, version, status).await?;
                            job = None;
                        }
                    }
                }
            }
            // Pick the next program from the queue.
            if job.is_none() {
                let program = {
                    let db = db.lock().await;
                    if let Some((tenant_id, program_id, version)) = db.next_job().await? {
                        trace!("Next program in the queue: '{program_id}', version '{version}'");
                        let program = db
                            .get_program_if_exists(tenant_id, program_id, true)
                            .await?;
                        Some((
                            tenant_id,
                            program_id,
                            version,
                            program.unwrap().code.unwrap(),
                        ))
                    } else {
                        None
                    }
                };

                if let Some((tenant_id, program_id, version, code)) = program {
                    job = Some(
                        CompilationJob::sql(tenant_id, &config, &code, program_id, version).await?,
                    );
                    db.lock()
                        .await
                        .set_program_status_guarded(
                            tenant_id,
                            program_id,
                            version,
                            ProgramStatus::CompilingSql,
                        )
                        .await?;
                }
            }
        }
    }
}

#[derive(Eq, PartialEq)]
enum Stage {
    Sql,
    Rust,
}

struct CompilationJob {
    stage: Stage,
    tenant_id: TenantId,
    program_id: ProgramId,
    version: Version,
    compiler_process: Child,
}

impl CompilationJob {
    fn is_sql(&self) -> bool {
        self.stage == Stage::Sql
    }

    fn is_rust(&self) -> bool {
        self.stage == Stage::Rust
    }

    /// Run SQL-to-DBSP compiler.
    async fn sql(
        tenant_id: TenantId,
        config: &CompilerConfig,
        code: &str,
        program_id: ProgramId,
        version: Version,
    ) -> Result<Self, ManagerError> {
        debug!("Running SQL compiler on program '{program_id}', version '{version}'");

        // Create project directory.
        let sql_file_path = config.sql_file_path(program_id);
        let project_directory = sql_file_path.parent().unwrap();
        fs::create_dir_all(&project_directory).await.map_err(|e| {
            ManagerError::io_error(
                format!(
                    "creating project directory '{}'",
                    project_directory.display()
                ),
                e,
            )
        })?;

        // Write SQL code to file.
        fs::write(&sql_file_path, code).await.map_err(|e| {
            ManagerError::io_error(format!("writing '{}'", sql_file_path.display()), e)
        })?;

        let rust_file_path = config.rust_program_path(program_id);
        let rust_source_dir = rust_file_path.parent().unwrap();
        fs::create_dir_all(&rust_source_dir).await.map_err(|e| {
            ManagerError::io_error(format!("creating '{}'", rust_source_dir.display()), e)
        })?;

        let stderr_path = config.compiler_stderr_path(program_id);
        let err_file = File::create(&stderr_path).await.map_err(|e| {
            ManagerError::io_error(format!("creating error log '{}'", stderr_path.display()), e)
        })?;

        // `main.rs` file.
        let rust_file = File::create(&rust_file_path).await.map_err(|e| {
            ManagerError::io_error(
                format!("failed to create '{}'", rust_file_path.display()),
                e,
            )
        })?;

        // Run compiler, direct output to `main.rs`.
        let schema_path = config.schema_path(program_id);
        let compiler_process = Command::new(config.sql_compiler_path())
            .arg("-js")
            .arg(schema_path)
            .arg(sql_file_path.as_os_str())
            .arg("-i")
            .arg("-je")
            .arg("-alltables")
            .stdin(Stdio::null())
            .stderr(Stdio::from(err_file.into_std().await))
            .stdout(Stdio::from(rust_file.into_std().await))
            .spawn()
            .map_err(|e| {
                ManagerError::io_error(
                    format!("starting SQL compiler '{}'", sql_file_path.display()),
                    e,
                )
            })?;

        Ok(Self {
            tenant_id,
            stage: Stage::Sql,
            program_id,
            version,
            compiler_process,
        })
    }

    // Run `cargo` on the generated Rust workspace.
    async fn rust(
        tenant_id: TenantId,
        config: &CompilerConfig,
        program_id: ProgramId,
        version: Version,
    ) -> Result<Self, ManagerError> {
        debug!("Running Rust compiler on program '{program_id}', version '{version}'");

        let rust_path = config.rust_program_path(program_id);
        let mut main_rs = OpenOptions::new()
            .append(true)
            .open(&rust_path)
            .await
            .map_err(|e| ManagerError::io_error(format!("opening '{}'", rust_path.display()), e))?;

        main_rs
            .write_all(MAIN_FUNCTION.as_bytes())
            .await
            .map_err(|e| ManagerError::io_error(format!("writing '{}'", rust_path.display()), e))?;
        drop(main_rs);

        // Write `project/Cargo.toml`.
        Compiler::write_project_toml(config, program_id).await?;

        // Write workspace `Cargo.toml`.  The workspace contains SQL libs and the
        // generated project crate.
        Compiler::write_workspace_toml(config, program_id).await?;

        // Run cargo, direct stdout and stderr to the same file.
        let compiler_process = Compiler::run_cargo_build(config, program_id).await?;

        Ok(Self {
            tenant_id,
            stage: Stage::Rust,
            program_id,
            version,
            compiler_process,
        })
    }

    /// Async-wait for the compiler to terminate.
    async fn wait(&mut self) -> Result<ExitStatus, ManagerError> {
        let exit_status = self.compiler_process.wait().await.map_err(|e| {
            ManagerError::io_error("waiting for the compiler process".to_string(), e)
        })?;
        Ok(exit_status)
        // doesn't update status
    }

    /// Read error output of (Rust or SQL) compiler.
    async fn error_output(&self, config: &CompilerConfig) -> Result<String, ManagerError> {
        let output = match self.stage {
            Stage::Sql => {
                let stderr_path = config.compiler_stderr_path(self.program_id);
                fs::read_to_string(&stderr_path).await.map_err(|e| {
                    ManagerError::io_error(format!("reading '{}'", stderr_path.display()), e)
                })?
            }
            Stage::Rust => {
                let stdout_path = config.compiler_stdout_path(self.program_id);
                let stdout = fs::read_to_string(&stdout_path).await.map_err(|e| {
                    ManagerError::io_error(format!("reading '{}'", stdout_path.display()), e)
                })?;
                let stderr_path = config.compiler_stderr_path(self.program_id);
                let stderr = fs::read_to_string(&stderr_path).await.map_err(|e| {
                    ManagerError::io_error(format!("reading '{}'", stderr_path.display()), e)
                })?;
                format!("stdout:\n{stdout}\nstderr:\n{stderr}")
            }
        };

        Ok(output)
    }

    /// Kill (Rust or SQL) compiler process.
    async fn cancel(&mut self) {
        let _ = self.compiler_process.kill().await;
    }
}

#[cfg(test)]
mod test {
    use std::{fs::File, sync::Arc};

    use tempfile::TempDir;
    use tokio::{fs, sync::Mutex};
    use uuid::Uuid;

    use crate::{
        auth::TenantRecord,
        compiler::ProgramStatus,
        config::CompilerConfig,
        db::{storage::Storage, ProgramId, ProjectDB, Version},
    };

    async fn create_program(db: &Arc<Mutex<ProjectDB>>, pname: &str) -> (ProgramId, Version) {
        let tenant_id = TenantRecord::default().id;
        db.lock()
            .await
            .new_program(tenant_id, Uuid::now_v7(), pname, "program desc", "ignored")
            .await
            .unwrap()
    }

    async fn check_program_status_pending(db: &Arc<Mutex<ProjectDB>>, pname: &str) {
        let tenant_id = TenantRecord::default().id;
        let programdesc = db
            .lock()
            .await
            .get_program_by_name(tenant_id, pname, false)
            .await
            .unwrap();
        assert_eq!(ProgramStatus::Pending, programdesc.status);
    }

    #[tokio::test]
    async fn test_compiler_reconcile_no_local_binary() {
        let tid = TenantRecord::default().id;
        let tmp_dir = TempDir::new().unwrap();
        let workdir = tmp_dir.path().to_str().unwrap();
        let conf = CompilerConfig {
            sql_compiler_home: "".to_owned(),
            dbsp_override_path: Some("../../".to_owned()),
            debug: false,
            precompile: false,
            compiler_working_directory: workdir.to_owned(),
        };

        let (db, _temp) = crate::db::test::setup_pg().await;
        let db = Arc::new(Mutex::new(db));

        // Empty binaries folder, no programs in API
        super::Compiler::reconcile_local_state(&conf, &db)
            .await
            .unwrap();

        // Create uncompiled program
        let (pid, vid) = create_program(&db, "p1").await;
        super::Compiler::reconcile_local_state(&conf, &db)
            .await
            .unwrap();

        // Now set the program to be queued for compilation
        db.lock()
            .await
            .set_program_for_compilation(tid, pid, vid, super::ProgramStatus::Pending)
            .await
            .unwrap();
        super::Compiler::reconcile_local_state(&conf, &db)
            .await
            .unwrap();
        check_program_status_pending(&db, "p1").await;

        // Now try all "compiling" states set the program to be compiled
        for state in vec![
            super::ProgramStatus::CompilingSql,
            super::ProgramStatus::CompilingRust,
            super::ProgramStatus::Success,
        ] {
            db.lock()
                .await
                .set_program_status_guarded(tid, pid, vid, state)
                .await
                .unwrap();
            super::Compiler::reconcile_local_state(&conf, &db)
                .await
                .unwrap();
            check_program_status_pending(&db, "p1").await;
        }
    }

    #[tokio::test]
    async fn test_compiler_with_local_binaries() {
        let tid = TenantRecord::default().id;
        let tmp_dir = TempDir::new().unwrap();
        let workdir = tmp_dir.path().to_str().unwrap();
        let conf = CompilerConfig {
            sql_compiler_home: "".to_owned(),
            dbsp_override_path: Some("../../".to_owned()),
            debug: false,
            precompile: false,
            compiler_working_directory: workdir.to_owned(),
        };

        let (db, _temp) = crate::db::test::setup_pg().await;
        let db = Arc::new(Mutex::new(db));

        let (pid1, v1) = create_program(&db, "p1").await;
        let (pid2, v2) = create_program(&db, "p2").await;
        for state in vec![
            super::ProgramStatus::Pending,
            super::ProgramStatus::CompilingSql,
        ] {
            db.lock()
                .await
                .set_program_status_guarded(tid, pid1, v1, state.clone())
                .await
                .unwrap();
            db.lock()
                .await
                .set_program_status_guarded(tid, pid2, v2, state)
                .await
                .unwrap();
            super::Compiler::reconcile_local_state(&conf, &db)
                .await
                .unwrap();
            check_program_status_pending(&db, "p1").await;
            check_program_status_pending(&db, "p2").await;
        }

        // Simulate compiled artifacts
        fs::create_dir(conf.binaries_dir()).await.unwrap();
        let path1 = conf.versioned_executable(pid1, v1);
        let path2 = conf.versioned_executable(pid2, v2);
        File::create(path1.clone()).unwrap();
        File::create(path2.clone()).unwrap();

        db.lock()
            .await
            .set_program_status_guarded(tid, pid1, v1, ProgramStatus::CompilingRust)
            .await
            .unwrap();
        db.lock()
            .await
            .set_program_status_guarded(tid, pid2, v2, ProgramStatus::CompilingRust)
            .await
            .unwrap();
        super::Compiler::reconcile_local_state(&conf, &db)
            .await
            .unwrap();
        check_program_status_pending(&db, "p1").await;
        check_program_status_pending(&db, "p2").await;
        assert!(!path1.exists());
        assert!(!path2.exists());
    }
}
