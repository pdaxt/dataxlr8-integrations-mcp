use dataxlr8_mcp_core::mcp::{error_result, get_i64, get_str, json_result, make_schema};
use dataxlr8_mcp_core::Database;
use rmcp::model::*;
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ServerHandler;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

// ============================================================================
// Data types
// ============================================================================

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct IntegrationConfig {
    pub id: String,
    pub platform: String,
    pub credentials: serde_json::Value,
    pub field_mapping: serde_json::Value,
    pub active: bool,
    pub last_sync: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SyncLogEntry {
    pub id: String,
    pub config_id: String,
    pub direction: String,
    pub records_synced: i32,
    pub errors: i32,
    pub details: String,
    pub synced_at: chrono::DateTime<chrono::Utc>,
}

/// Redacted view of config for list output (hides credentials).
#[derive(Debug, Serialize)]
pub struct IntegrationSummary {
    pub id: String,
    pub platform: String,
    pub active: bool,
    pub has_credentials: bool,
    pub field_mapping: serde_json::Value,
    pub last_sync: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ============================================================================
// Constants & validation helpers
// ============================================================================

const VALID_PLATFORMS: &[&str] = &["linkedin", "seek", "indeed", "xing"];
const DEFAULT_LIMIT: i64 = 50;
const DEFAULT_OFFSET: i64 = 0;
const MAX_LIMIT: i64 = 500;

/// Clamp limit to [1, MAX_LIMIT], defaulting to `default`.
fn clamp_limit(args: &serde_json::Value, key: &str, default: i64) -> i64 {
    get_i64(args, key).unwrap_or(default).clamp(1, MAX_LIMIT)
}

/// Clamp offset to >= 0, defaulting to 0.
fn clamp_offset(args: &serde_json::Value) -> i64 {
    get_i64(args, "offset").unwrap_or(DEFAULT_OFFSET).max(0)
}

/// Extract and trim a required string parameter. Returns Err(CallToolResult) on missing/empty.
fn require_trimmed_str(args: &serde_json::Value, key: &str) -> Result<String, CallToolResult> {
    match get_str(args, key) {
        Some(s) => {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                Err(error_result(&format!("Parameter '{}' must not be empty", key)))
            } else {
                Ok(trimmed)
            }
        }
        None => Err(error_result(&format!("Missing required parameter: {}", key))),
    }
}

/// Extract and validate a required JSON object parameter.
fn require_json_object(args: &serde_json::Value, key: &str) -> Result<serde_json::Value, CallToolResult> {
    match args.get(key) {
        Some(v) if v.is_object() => {
            let obj = v.as_object().unwrap();
            if obj.is_empty() {
                Err(error_result(&format!("Parameter '{}' must be a non-empty JSON object", key)))
            } else {
                Ok(v.clone())
            }
        }
        Some(_) => Err(error_result(&format!("Parameter '{}' must be a JSON object", key))),
        None => Err(error_result(&format!("Missing required parameter: {}", key))),
    }
}

// ============================================================================
// Tool definitions
// ============================================================================

fn build_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "configure_integration".into(),
            title: None,
            description: Some(
                "Set up API keys/credentials for an external platform (linkedin, seek, indeed, xing). Credentials stored as JSONB."
                    .into(),
            ),
            input_schema: make_schema(
                serde_json::json!({
                    "platform": { "type": "string", "enum": ["linkedin", "seek", "indeed", "xing"], "description": "Platform to configure" },
                    "credentials": { "type": "object", "description": "API keys/tokens as JSON object (e.g. {\"api_key\": \"...\", \"secret\": \"...\"})" },
                    "field_mapping": { "type": "object", "description": "Optional field mapping between platform and dataxlr8 schema" }
                }),
                vec!["platform", "credentials"],
            ),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "list_integrations".into(),
            title: None,
            description: Some(
                "Show all configured integrations and their status. Credentials are redacted. Supports pagination via limit/offset."
                    .into(),
            ),
            input_schema: make_schema(
                serde_json::json!({
                    "limit": { "type": "integer", "description": "Max results to return (default: 50, max: 500)" },
                    "offset": { "type": "integer", "description": "Number of results to skip (default: 0)" }
                }),
                vec![],
            ),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "sync_contacts".into(),
            title: None,
            description: Some(
                "Pull contacts from an integration into CRM format. Records a sync log entry with counts."
                    .into(),
            ),
            input_schema: make_schema(
                serde_json::json!({
                    "platform": { "type": "string", "enum": ["linkedin", "seek", "indeed", "xing"], "description": "Platform to sync from" },
                    "limit": { "type": "integer", "description": "Max records to sync (default: 50, max: 500)" }
                }),
                vec!["platform"],
            ),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "push_candidate".into(),
            title: None,
            description: Some(
                "Send candidate data to a job board. Records a sync log entry."
                    .into(),
            ),
            input_schema: make_schema(
                serde_json::json!({
                    "platform": { "type": "string", "enum": ["linkedin", "seek", "indeed", "xing"], "description": "Target job board" },
                    "candidate": { "type": "object", "description": "Candidate data object with name, email, skills, etc." }
                }),
                vec!["platform", "candidate"],
            ),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "check_status".into(),
            title: None,
            description: Some(
                "Verify integration connectivity — checks if credentials are configured and the integration is active."
                    .into(),
            ),
            input_schema: make_schema(
                serde_json::json!({
                    "platform": { "type": "string", "enum": ["linkedin", "seek", "indeed", "xing"], "description": "Platform to check" }
                }),
                vec!["platform"],
            ),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "integration_log".into(),
            title: None,
            description: Some(
                "Show sync history for an integration. Returns recent sync log entries with pagination."
                    .into(),
            ),
            input_schema: make_schema(
                serde_json::json!({
                    "platform": { "type": "string", "enum": ["linkedin", "seek", "indeed", "xing"], "description": "Platform to view logs for" },
                    "limit": { "type": "integer", "description": "Max log entries to return (default: 50, max: 500)" },
                    "offset": { "type": "integer", "description": "Number of entries to skip (default: 0)" }
                }),
                vec!["platform"],
            ),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "map_fields".into(),
            title: None,
            description: Some(
                "Define field mapping between an external platform's schema and the dataxlr8 schema. Updates the integration config."
                    .into(),
            ),
            input_schema: make_schema(
                serde_json::json!({
                    "platform": { "type": "string", "enum": ["linkedin", "seek", "indeed", "xing"], "description": "Platform to map fields for" },
                    "mapping": { "type": "object", "description": "Field mapping object, e.g. {\"first_name\": \"firstName\", \"email\": \"emailAddress\"}" }
                }),
                vec!["platform", "mapping"],
            ),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "disable_integration".into(),
            title: None,
            description: Some(
                "Pause an integration by setting it to inactive. Can be re-enabled with configure_integration."
                    .into(),
            ),
            input_schema: make_schema(
                serde_json::json!({
                    "platform": { "type": "string", "enum": ["linkedin", "seek", "indeed", "xing"], "description": "Platform to disable" }
                }),
                vec!["platform"],
            ),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        },
    ]
}

// ============================================================================
// MCP Server
// ============================================================================

#[derive(Clone)]
pub struct IntegrationsMcpServer {
    db: Database,
}

impl IntegrationsMcpServer {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    fn validate_platform(platform: &str) -> Result<(), CallToolResult> {
        if VALID_PLATFORMS.contains(&platform) {
            Ok(())
        } else {
            Err(error_result(&format!(
                "Invalid platform '{}'. Must be one of: {}",
                platform,
                VALID_PLATFORMS.join(", ")
            )))
        }
    }

    /// Extract, trim, and validate a platform parameter from args.
    fn require_platform(args: &serde_json::Value) -> Result<String, CallToolResult> {
        let platform = require_trimmed_str(args, "platform")?;
        Self::validate_platform(&platform)?;
        Ok(platform)
    }

    // ---- Tool handlers ----

    async fn handle_configure_integration(&self, args: &serde_json::Value) -> CallToolResult {
        let platform = match Self::require_platform(args) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let credentials = match require_json_object(args, "credentials") {
            Ok(c) => c,
            Err(e) => return e,
        };

        let field_mapping = args
            .get("field_mapping")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let id = uuid::Uuid::new_v4().to_string();

        match sqlx::query_as::<_, IntegrationConfig>(
            r#"INSERT INTO integrations.configs (id, platform, credentials, field_mapping, active)
               VALUES ($1, $2, $3, $4, true)
               ON CONFLICT (platform)
               DO UPDATE SET credentials = EXCLUDED.credentials,
                             field_mapping = CASE
                                 WHEN EXCLUDED.field_mapping != '{}'::jsonb THEN EXCLUDED.field_mapping
                                 ELSE integrations.configs.field_mapping
                             END,
                             active = true,
                             updated_at = now()
               RETURNING *"#,
        )
        .bind(&id)
        .bind(&platform)
        .bind(&credentials)
        .bind(&field_mapping)
        .fetch_one(self.db.pool())
        .await
        {
            Ok(config) => {
                info!(platform = %platform, "Configured integration");
                json_result(&IntegrationSummary {
                    id: config.id,
                    platform: config.platform,
                    active: config.active,
                    has_credentials: true,
                    field_mapping: config.field_mapping,
                    last_sync: config.last_sync,
                    created_at: config.created_at,
                })
            }
            Err(e) => {
                error!(platform = %platform, error = %e, "Failed to configure integration");
                error_result(&format!("Failed to configure integration: {e}"))
            }
        }
    }

    async fn handle_list_integrations(&self, args: &serde_json::Value) -> CallToolResult {
        let limit = clamp_limit(args, "limit", DEFAULT_LIMIT);
        let offset = clamp_offset(args);

        let configs: Vec<IntegrationConfig> = match sqlx::query_as(
            "SELECT * FROM integrations.configs ORDER BY platform LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(self.db.pool())
        .await
        {
            Ok(c) => c,
            Err(e) => {
                error!(error = %e, "Failed to list integrations");
                return error_result(&format!("Database error: {e}"));
            }
        };

        let summaries: Vec<IntegrationSummary> = configs
            .into_iter()
            .map(|c| {
                let has_creds = c.credentials.as_object().map_or(false, |o| !o.is_empty());
                IntegrationSummary {
                    id: c.id,
                    platform: c.platform,
                    active: c.active,
                    has_credentials: has_creds,
                    field_mapping: c.field_mapping,
                    last_sync: c.last_sync,
                    created_at: c.created_at,
                }
            })
            .collect();

        json_result(&serde_json::json!({
            "integrations": summaries,
            "count": summaries.len(),
            "limit": limit,
            "offset": offset
        }))
    }

    async fn handle_sync_contacts(&self, args: &serde_json::Value) -> CallToolResult {
        let platform = match Self::require_platform(args) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let limit = clamp_limit(args, "limit", DEFAULT_LIMIT);

        // Verify integration exists and is active
        let config: Option<IntegrationConfig> = match sqlx::query_as(
            "SELECT * FROM integrations.configs WHERE platform = $1",
        )
        .bind(&platform)
        .fetch_optional(self.db.pool())
        .await
        {
            Ok(c) => c,
            Err(e) => {
                error!(platform = %platform, error = %e, "Failed to fetch integration config for sync");
                return error_result(&format!("Database error: {e}"));
            }
        };

        let config = match config {
            Some(c) if c.active => c,
            Some(_) => return error_result(&format!("Integration '{}' is disabled", platform)),
            None => return error_result(&format!("Integration '{}' not configured. Use configure_integration first.", platform)),
        };

        // Simulate sync -- in production this would call the platform's API
        let sync_id = uuid::Uuid::new_v4().to_string();
        let details = format!("Pull sync requested for up to {} contacts from {}", limit, platform);

        match sqlx::query_as::<_, SyncLogEntry>(
            r#"INSERT INTO integrations.sync_log (id, config_id, direction, records_synced, errors, details)
               VALUES ($1, $2, 'pull', 0, 0, $3)
               RETURNING *"#,
        )
        .bind(&sync_id)
        .bind(&config.id)
        .bind(&details)
        .fetch_one(self.db.pool())
        .await
        {
            Ok(log) => {
                if let Err(e) = sqlx::query("UPDATE integrations.configs SET last_sync = now(), updated_at = now() WHERE id = $1")
                    .bind(&config.id)
                    .execute(self.db.pool())
                    .await
                {
                    error!(config_id = %config.id, error = %e, "Failed to update last_sync timestamp");
                }

                info!(platform = %platform, limit = limit, "Sync contacts initiated");
                json_result(&serde_json::json!({
                    "status": "sync_initiated",
                    "platform": platform,
                    "limit": limit,
                    "sync_log": log,
                    "note": "Connect platform API adapter to perform actual data pull"
                }))
            }
            Err(e) => {
                error!(platform = %platform, error = %e, "Failed to record sync");
                error_result(&format!("Failed to record sync: {e}"))
            }
        }
    }

    async fn handle_push_candidate(&self, args: &serde_json::Value) -> CallToolResult {
        let platform = match Self::require_platform(args) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let candidate = match require_json_object(args, "candidate") {
            Ok(c) => c,
            Err(e) => return e,
        };

        // Verify integration exists and is active
        let config: Option<IntegrationConfig> = match sqlx::query_as(
            "SELECT * FROM integrations.configs WHERE platform = $1 AND active = true",
        )
        .bind(&platform)
        .fetch_optional(self.db.pool())
        .await
        {
            Ok(c) => c,
            Err(e) => {
                error!(platform = %platform, error = %e, "Failed to fetch integration config for push");
                return error_result(&format!("Database error: {e}"));
            }
        };

        let config = match config {
            Some(c) => c,
            None => return error_result(&format!("Integration '{}' not configured or disabled", platform)),
        };

        // Record the push attempt
        let sync_id = uuid::Uuid::new_v4().to_string();
        let candidate_name = candidate
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .unwrap_or("unknown");
        let details = format!("Push candidate '{}' to {}", candidate_name, platform);

        match sqlx::query_as::<_, SyncLogEntry>(
            r#"INSERT INTO integrations.sync_log (id, config_id, direction, records_synced, errors, details)
               VALUES ($1, $2, 'push', 1, 0, $3)
               RETURNING *"#,
        )
        .bind(&sync_id)
        .bind(&config.id)
        .bind(&details)
        .fetch_one(self.db.pool())
        .await
        {
            Ok(log) => {
                if let Err(e) = sqlx::query("UPDATE integrations.configs SET last_sync = now(), updated_at = now() WHERE id = $1")
                    .bind(&config.id)
                    .execute(self.db.pool())
                    .await
                {
                    error!(config_id = %config.id, error = %e, "Failed to update last_sync timestamp");
                }

                info!(platform = %platform, candidate = candidate_name, "Push candidate initiated");
                json_result(&serde_json::json!({
                    "status": "push_initiated",
                    "platform": platform,
                    "candidate": candidate,
                    "sync_log": log,
                    "note": "Connect platform API adapter to perform actual push"
                }))
            }
            Err(e) => {
                error!(platform = %platform, error = %e, "Failed to record push");
                error_result(&format!("Failed to record push: {e}"))
            }
        }
    }

    async fn handle_check_status(&self, args: &serde_json::Value) -> CallToolResult {
        let platform = match Self::require_platform(args) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let config: Option<IntegrationConfig> = match sqlx::query_as(
            "SELECT * FROM integrations.configs WHERE platform = $1",
        )
        .bind(&platform)
        .fetch_optional(self.db.pool())
        .await
        {
            Ok(c) => c,
            Err(e) => {
                error!(platform = %platform, error = %e, "Failed to check integration status");
                return error_result(&format!("Database error: {e}"));
            }
        };

        match config {
            Some(c) => {
                let has_creds = c.credentials.as_object().map_or(false, |o| !o.is_empty());
                json_result(&serde_json::json!({
                    "platform": c.platform,
                    "configured": true,
                    "active": c.active,
                    "has_credentials": has_creds,
                    "has_field_mapping": c.field_mapping.as_object().map_or(false, |o| !o.is_empty()),
                    "last_sync": c.last_sync,
                    "status": if c.active && has_creds { "ready" } else if !c.active { "disabled" } else { "missing_credentials" }
                }))
            }
            None => {
                json_result(&serde_json::json!({
                    "platform": platform,
                    "configured": false,
                    "active": false,
                    "status": "not_configured"
                }))
            }
        }
    }

    async fn handle_integration_log(&self, args: &serde_json::Value) -> CallToolResult {
        let platform = match Self::require_platform(args) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let limit = clamp_limit(args, "limit", DEFAULT_LIMIT);
        let offset = clamp_offset(args);

        let logs: Vec<SyncLogEntry> = match sqlx::query_as(
            r#"SELECT sl.* FROM integrations.sync_log sl
               JOIN integrations.configs c ON sl.config_id = c.id
               WHERE c.platform = $1
               ORDER BY sl.synced_at DESC
               LIMIT $2 OFFSET $3"#,
        )
        .bind(&platform)
        .bind(limit)
        .bind(offset)
        .fetch_all(self.db.pool())
        .await
        {
            Ok(l) => l,
            Err(e) => {
                error!(platform = %platform, error = %e, "Failed to fetch integration logs");
                return error_result(&format!("Database error: {e}"));
            }
        };

        json_result(&serde_json::json!({
            "platform": platform,
            "count": logs.len(),
            "limit": limit,
            "offset": offset,
            "logs": logs
        }))
    }

    async fn handle_map_fields(&self, args: &serde_json::Value) -> CallToolResult {
        let platform = match Self::require_platform(args) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let mapping = match require_json_object(args, "mapping") {
            Ok(m) => m,
            Err(e) => return e,
        };

        match sqlx::query_as::<_, IntegrationConfig>(
            r#"UPDATE integrations.configs
               SET field_mapping = $1, updated_at = now()
               WHERE platform = $2
               RETURNING *"#,
        )
        .bind(&mapping)
        .bind(&platform)
        .fetch_optional(self.db.pool())
        .await
        {
            Ok(Some(config)) => {
                info!(platform = %platform, "Updated field mapping");
                json_result(&serde_json::json!({
                    "platform": config.platform,
                    "field_mapping": config.field_mapping,
                    "updated": true
                }))
            }
            Ok(None) => error_result(&format!("Integration '{}' not configured. Use configure_integration first.", platform)),
            Err(e) => {
                error!(platform = %platform, error = %e, "Failed to update field mapping");
                error_result(&format!("Failed to update field mapping: {e}"))
            }
        }
    }

    async fn handle_disable_integration(&self, args: &serde_json::Value) -> CallToolResult {
        let platform = match Self::require_platform(args) {
            Ok(p) => p,
            Err(e) => return e,
        };

        match sqlx::query("UPDATE integrations.configs SET active = false, updated_at = now() WHERE platform = $1")
            .bind(&platform)
            .execute(self.db.pool())
            .await
        {
            Ok(r) => {
                if r.rows_affected() > 0 {
                    info!(platform = %platform, "Disabled integration");
                    json_result(&serde_json::json!({
                        "platform": platform,
                        "active": false,
                        "disabled": true
                    }))
                } else {
                    error_result(&format!("Integration '{}' not configured", platform))
                }
            }
            Err(e) => {
                error!(platform = %platform, error = %e, "Failed to disable integration");
                error_result(&format!("Failed to disable integration: {e}"))
            }
        }
    }
}

// ============================================================================
// ServerHandler trait implementation
// ============================================================================

impl ServerHandler for IntegrationsMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "DataXLR8 Integrations MCP — connect to external recruitment platforms (LinkedIn, Seek, Indeed, XING)"
                    .into(),
            ),
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_ {
        async {
            Ok(ListToolsResult {
                tools: build_tools(),
                next_cursor: None,
                meta: None,
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::ErrorData>> + Send + '_ {
        async move {
            let args = serde_json::to_value(&request.arguments).unwrap_or(serde_json::Value::Null);
            let name_str: &str = request.name.as_ref();

            let result = match name_str {
                "configure_integration" => self.handle_configure_integration(&args).await,
                "list_integrations" => self.handle_list_integrations(&args).await,
                "sync_contacts" => self.handle_sync_contacts(&args).await,
                "push_candidate" => self.handle_push_candidate(&args).await,
                "check_status" => self.handle_check_status(&args).await,
                "integration_log" => self.handle_integration_log(&args).await,
                "map_fields" => self.handle_map_fields(&args).await,
                "disable_integration" => self.handle_disable_integration(&args).await,
                _ => error_result(&format!("Unknown tool: {}", request.name)),
            };

            Ok(result)
        }
    }
}
