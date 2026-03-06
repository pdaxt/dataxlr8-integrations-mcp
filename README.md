# dataxlr8-integrations-mcp

Manages integrations with external recruitment platforms (LinkedIn, Seek, Indeed, Xing). Handles credential configuration, field mapping, contact synchronization, and candidate push operations.

## Tools

| Tool | Description |
|------|-------------|
| configure_integration | Set up API keys/credentials for an external platform (linkedin, seek, indeed, xing). Credentials stored as JSONB. |
| list_integrations | Show all configured integrations and their status. Credentials are redacted. Supports pagination via limit/offset. |
| sync_contacts | Pull contacts from an integration into CRM format. Records a sync log entry with counts. |
| push_candidate | Send candidate data to a job board. Records a sync log entry. |
| check_status | Verify integration connectivity — checks if credentials are configured and the integration is active. |
| integration_log | Show sync history for an integration. Returns recent sync log entries with pagination. |
| map_fields | Define field mapping between an external platform's schema and the dataxlr8 schema. Updates the integration config. |
| disable_integration | Pause an integration by setting it to inactive. Can be re-enabled with configure_integration. |

## Setup

```bash
DATABASE_URL=postgres://dataxlr8:dataxlr8@localhost:5432/dataxlr8 cargo run
```

## Schema

Creates `integrations` schema in PostgreSQL with tables for platform configurations and sync logs.

## Part of

[DataXLR8](https://github.com/pdaxt) - AI-powered recruitment platform
