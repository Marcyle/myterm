use anyhow::{Context as _, Result};
use chrono::Utc;
use one_core::storage::{ConnectionType, SshAuthMethod, SshParams, StoredConnection, Workspace};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportScope {
    #[default]
    All,
    CurrentGroup,
    Selected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportFormat {
    #[default]
    Json,
    Csv,
    Yaml,
}

impl ExportFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Yaml => "yaml",
        }
    }

    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Csv => "CSV",
            Self::Yaml => "YAML",
        }
    }
}

/// 导出白名单结构体（支持脱敏与明文凭据导出）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportConnectionItem {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub protocol: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportPayload {
    pub app: String,
    pub version: u32,
    #[serde(rename = "exportedAt")]
    pub exported_at: String,
    pub masked: bool,
    pub connections: Vec<ExportConnectionItem>,
}

impl ExportConnectionItem {
    pub fn unique_key(&self) -> String {
        format!("{}:{}:{}", self.host, self.port, self.user)
    }
}

/// 从 StoredConnection 构造导出条目
pub fn build_export_item(
    conn: &StoredConnection,
    workspaces: &[Workspace],
    include_passwords: bool,
) -> ExportConnectionItem {
    let group = conn.workspace_id.and_then(|wid| {
        workspaces
            .iter()
            .find(|w| w.id == Some(wid))
            .map(|w| w.name.clone())
    });

    let (user, host, port) = conn.endpoint_display();
    let protocol = match conn.connection_type {
        ConnectionType::SshSftp => "SSH".to_string(),
        ConnectionType::PortForwarding => "PortForwarding".to_string(),
        ConnectionType::Jms => "JMS".to_string(),
        ConnectionType::All => "All".to_string(),
    };

    let password = if include_passwords {
        match conn.connection_type {
            ConnectionType::SshSftp => {
                if let Ok(params) = conn.to_ssh_params() {
                    match params.auth_method {
                        SshAuthMethod::Password { password } => {
                            if !password.is_empty() {
                                Some(password)
                            } else {
                                None
                            }
                        }
                        SshAuthMethod::PrivateKey { passphrase, .. } => passphrase,
                        _ => None,
                    }
                } else {
                    None
                }
            }
            ConnectionType::Jms => {
                if let Ok(params) = conn.to_jms_params() {
                    if !params.password.is_empty() {
                        Some(params.password)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    } else {
        None
    };

    ExportConnectionItem {
        name: conn.name.clone(),
        group,
        protocol,
        user,
        host,
        port,
        password,
        env: conn.env.clone(),
        tags: conn.tags_vec(),
    }
}

/// 生成导出内容
pub fn generate_export_content(
    items: Vec<ExportConnectionItem>,
    format: ExportFormat,
    include_passwords: bool,
) -> Result<String> {
    match format {
        ExportFormat::Json => {
            let payload = ExportPayload {
                app: "MyTerm".to_string(),
                version: 2,
                exported_at: Utc::now().to_rfc3339(),
                masked: !include_passwords,
                connections: items,
            };
            Ok(serde_json::to_string_pretty(&payload)?)
        }
        ExportFormat::Csv => {
            let mut wtr = String::new();
            if include_passwords {
                wtr.push_str("name,group,protocol,user,host,port,password,env,tags\n");
                for row in items {
                    let tags_str = row.tags.join("|");
                    let pwd_str = row.password.unwrap_or_default().replace('"', "\"\"");
                    let line = format!(
                        "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",{},\"{}\",\"{}\",\"{}\"\n",
                        row.name.replace('"', "\"\""),
                        row.group.unwrap_or_default().replace('"', "\"\""),
                        row.protocol,
                        row.user.replace('"', "\"\""),
                        row.host.replace('"', "\"\""),
                        row.port,
                        pwd_str,
                        row.env.unwrap_or_default().replace('"', "\"\""),
                        tags_str.replace('"', "\"\"")
                    );
                    wtr.push_str(&line);
                }
            } else {
                wtr.push_str("name,group,protocol,user,host,port,env,tags\n");
                for row in items {
                    let tags_str = row.tags.join("|");
                    let line = format!(
                        "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",{},\"{}\",\"{}\"\n",
                        row.name.replace('"', "\"\""),
                        row.group.unwrap_or_default().replace('"', "\"\""),
                        row.protocol,
                        row.user.replace('"', "\"\""),
                        row.host.replace('"', "\"\""),
                        row.port,
                        row.env.unwrap_or_default().replace('"', "\"\""),
                        tags_str.replace('"', "\"\"")
                    );
                    wtr.push_str(&line);
                }
            }
            Ok(wtr)
        }
        ExportFormat::Yaml => {
            let mut yml = String::new();
            yml.push_str(&format!(
                "app: MyTerm\nversion: 2\nmasked: {}\nconnections:\n",
                !include_passwords
            ));
            for r in items {
                let env_str = r
                    .env
                    .map(|e| format!("\"{}\"", e))
                    .unwrap_or_else(|| "null".to_string());
                let group_str = r
                    .group
                    .map(|g| format!("\"{}\"", g))
                    .unwrap_or_else(|| "null".to_string());
                let tags_str = r
                    .tags
                    .iter()
                    .map(|t| format!("\"{}\"", t))
                    .collect::<Vec<_>>()
                    .join(", ");
                if include_passwords {
                    let pwd_str = r
                        .password
                        .map(|p| format!("\"{}\"", p.replace('"', "\\\"")))
                        .unwrap_or_else(|| "null".to_string());
                    yml.push_str(&format!(
                        "  - {{name: \"{}\", group: {}, protocol: \"{}\", user: \"{}\", host: \"{}\", port: {}, password: {}, env: {}, tags: [{}]}}\n",
                        r.name.replace('"', "\\\""),
                        group_str,
                        r.protocol,
                        r.user,
                        r.host,
                        r.port,
                        pwd_str,
                        env_str,
                        tags_str
                    ));
                } else {
                    yml.push_str(&format!(
                        "  - {{name: \"{}\", group: {}, protocol: \"{}\", user: \"{}\", host: \"{}\", port: {}, env: {}, tags: [{}]}}\n",
                        r.name.replace('"', "\\\""),
                        group_str,
                        r.protocol,
                        r.user,
                        r.host,
                        r.port,
                        env_str,
                        tags_str
                    ));
                }
            }
            Ok(yml)
        }
    }
}

/// 保存导出文件至下载目录或临时目录
pub fn save_export_file(content: &str, format: ExportFormat) -> Result<PathBuf> {
    let filename = format!("myterm-connections.{}", format.extension());
    let target_dir = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|p| p.join("Downloads"))
        .filter(|p| p.exists())
        .unwrap_or_else(std::env::temp_dir);

    let target_path = target_dir.join(filename);
    std::fs::write(&target_path, content)
        .with_context(|| format!("写入导出文件失败: {}", target_path.display()))?;
    Ok(target_path)
}

/* ================= 导入逻辑与合并策略 ================= */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeStrategy {
    #[default]
    KeepLocal,
    Overwrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportCategory {
    New,
    Conflict {
        local_name: String,
        diff_desc: String,
    },
    Same,
}

#[derive(Debug, Clone)]
pub struct ImportPreflightItem {
    pub item: ExportConnectionItem,
    pub category: ImportCategory,
    pub local_conn_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ImportPreflightReport {
    pub items: Vec<ImportPreflightItem>,
    pub total: usize,
    pub new_count: usize,
    pub conflict_count: usize,
    pub same_count: usize,
}

/// 解析导入文件内容（支持 JSON / CSV）
pub fn parse_import_payload(text: &str) -> Result<Vec<ExportConnectionItem>> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        // Full ExportPayload
        let payload: ExportPayload =
            serde_json::from_str(trimmed).context("JSON 格式不符合 MyTerm 导出规范")?;
        return Ok(payload.connections);
    }
    if trimmed.starts_with('[') {
        // Direct array of items
        let items: Vec<ExportConnectionItem> =
            serde_json::from_str(trimmed).context("JSON 数组解析失败")?;
        return Ok(items);
    }

    // Try CSV
    let first_line = trimmed.lines().next().unwrap_or_default();
    if first_line.contains("host") && first_line.contains("port") {
        let has_pwd = first_line.contains("password");
        let mut items = Vec::new();
        for (idx, line) in trimmed.lines().enumerate() {
            if idx == 0 || line.trim().is_empty() {
                continue;
            }
            let cols: Vec<&str> = line
                .split(',')
                .map(|s| s.trim().trim_matches('"'))
                .collect();
            if cols.len() >= 6 {
                let name = cols[0].to_string();
                let group = if cols[1].is_empty() {
                    None
                } else {
                    Some(cols[1].to_string())
                };
                let protocol = cols[2].to_string();
                let user = cols[3].to_string();
                let host = cols[4].to_string();
                let port = cols[5].parse::<u16>().unwrap_or(22);
                let (password, env, tags) = if has_pwd {
                    let pwd = if cols.len() > 6 && !cols[6].is_empty() {
                        Some(cols[6].to_string())
                    } else {
                        None
                    };
                    let env = if cols.len() > 7 && !cols[7].is_empty() {
                        Some(cols[7].to_string())
                    } else {
                        None
                    };
                    let tags = if cols.len() > 8 && !cols[8].is_empty() {
                        cols[8]
                            .split('|')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    } else {
                        Vec::new()
                    };
                    (pwd, env, tags)
                } else {
                    let env = if cols.len() > 6 && !cols[6].is_empty() {
                        Some(cols[6].to_string())
                    } else {
                        None
                    };
                    let tags = if cols.len() > 7 && !cols[7].is_empty() {
                        cols[7]
                            .split('|')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    } else {
                        Vec::new()
                    };
                    (None, env, tags)
                };

                items.push(ExportConnectionItem {
                    name,
                    group,
                    protocol,
                    user,
                    host,
                    port,
                    password,
                    env,
                    tags,
                });
            }
        }
        if !items.is_empty() {
            return Ok(items);
        }
    }

    anyhow::bail!("无法识别的文件格式，需要 MyTerm 导出的 .json 或 .csv 配置文件")
}

/// 示例演示文件数据（包含 1 新增、1 冲突、1 相同）
pub fn sample_import_json() -> &'static str {
    r#"{
  "app": "MyTerm",
  "version": 2,
  "exportedAt": "2026-03-31T12:00:00Z",
  "masked": true,
  "connections": [
    {
      "name": "redis-cache-5.120",
      "group": "TXXY",
      "protocol": "SSH",
      "user": "root",
      "host": "10.196.5.120",
      "port": 22,
      "env": "prod",
      "tags": ["中间件"]
    },
    {
      "name": "k8s-master-1（HA 改造后改名）",
      "group": "PyCredit",
      "protocol": "SSH",
      "user": "root",
      "host": "10.196.2.55",
      "port": 22,
      "env": "dev",
      "tags": ["k8s", "搜索", "HA"]
    },
    {
      "name": "4楼测试机",
      "group": "Personal",
      "protocol": "SSH",
      "user": "root",
      "host": "10.196.5.29",
      "port": 22,
      "env": "test",
      "tags": ["个人"]
    }
  ]
}"#
}

/// 预检比较：以 host:port:user 为唯一键比对本地连接
pub fn preflight_import(
    imported: Vec<ExportConnectionItem>,
    local_connections: &[StoredConnection],
) -> ImportPreflightReport {
    let mut report = ImportPreflightReport {
        total: imported.len(),
        ..Default::default()
    };

    for item in imported {
        let item_key = item.unique_key();
        let matched_local = local_connections.iter().find(|loc| {
            let (u, h, p) = loc.endpoint_display();
            format!("{}:{}:{}", h, p, u) == item_key
        });

        if let Some(local) = matched_local {
            let mut diffs = Vec::new();
            if local.name != item.name {
                diffs.push(format!("名称变更: \"{}\" → \"{}\"", local.name, item.name));
            }
            if local.env != item.env {
                diffs.push(format!(
                    "环境变更: {:?} → {:?}",
                    local.env.as_deref().unwrap_or("无"),
                    item.env.as_deref().unwrap_or("无")
                ));
            }
            let local_tags = local.tags_vec();
            if local_tags != item.tags {
                diffs.push("标签差异".to_string());
            }

            if diffs.is_empty() {
                report.same_count += 1;
                report.items.push(ImportPreflightItem {
                    item,
                    category: ImportCategory::Same,
                    local_conn_id: local.id,
                });
            } else {
                report.conflict_count += 1;
                report.items.push(ImportPreflightItem {
                    item,
                    category: ImportCategory::Conflict {
                        local_name: local.name.clone(),
                        diff_desc: diffs.join(" · "),
                    },
                    local_conn_id: local.id,
                });
            }
        } else {
            report.new_count += 1;
            report.items.push(ImportPreflightItem {
                item,
                category: ImportCategory::New,
                local_conn_id: None,
            });
        }
    }

    report
}

/// 生成待创建与待更新的 StoredConnection 实体（纯内存计算，不写库）
pub fn plan_import_execution(
    report: &ImportPreflightReport,
    strategy: MergeStrategy,
    workspaces: &[Workspace],
) -> (Vec<StoredConnection>, Vec<(i64, StoredConnection)>) {
    let mut to_create = Vec::new();
    let mut to_update = Vec::new();

    for preflight in &report.items {
        match &preflight.category {
            ImportCategory::New => {
                let ws_id = preflight.item.group.as_ref().and_then(|gname| {
                    workspaces
                        .iter()
                        .find(|w| &w.name == gname)
                        .and_then(|w| w.id)
                });
                let auth_method = if let Some(pwd) = &preflight.item.password {
                    if !pwd.is_empty() {
                        SshAuthMethod::Password {
                            password: pwd.clone(),
                        }
                    } else {
                        SshAuthMethod::Agent
                    }
                } else {
                    SshAuthMethod::Agent
                };
                let params = SshParams {
                    host: preflight.item.host.clone(),
                    port: preflight.item.port,
                    username: preflight.item.user.clone(),
                    auth_method,
                    connect_timeout: None,
                    keepalive_interval: None,
                    keepalive_max: None,
                    default_directory: None,
                    init_script: None,
                    disable_shell_integration: None,
                    jump_server: None,
                    proxy: None,
                };
                let mut conn =
                    StoredConnection::new_ssh(preflight.item.name.clone(), params, ws_id);
                conn.env = preflight.item.env.clone();
                conn.tags = if preflight.item.tags.is_empty() {
                    None
                } else {
                    Some(preflight.item.tags.clone())
                };
                to_create.push(conn);
            }
            ImportCategory::Conflict { .. } => {
                if strategy == MergeStrategy::Overwrite {
                    if let Some(local_id) = preflight.local_conn_id {
                        let ws_id = preflight.item.group.as_ref().and_then(|gname| {
                            workspaces
                                .iter()
                                .find(|w| &w.name == gname)
                                .and_then(|w| w.id)
                        });
                        let auth_method = if let Some(pwd) = &preflight.item.password {
                            if !pwd.is_empty() {
                                SshAuthMethod::Password {
                                    password: pwd.clone(),
                                }
                            } else {
                                SshAuthMethod::Agent
                            }
                        } else {
                            SshAuthMethod::Agent
                        };
                        let params = SshParams {
                            host: preflight.item.host.clone(),
                            port: preflight.item.port,
                            username: preflight.item.user.clone(),
                            auth_method,
                            connect_timeout: None,
                            keepalive_interval: None,
                            keepalive_max: None,
                            default_directory: None,
                            init_script: None,
                            disable_shell_integration: None,
                            jump_server: None,
                            proxy: None,
                        };
                        let mut conn =
                            StoredConnection::new_ssh(preflight.item.name.clone(), params, ws_id);
                        conn.id = Some(local_id);
                        conn.env = preflight.item.env.clone();
                        conn.tags = if preflight.item.tags.is_empty() {
                            None
                        } else {
                            Some(preflight.item.tags.clone())
                        };
                        to_update.push((local_id, conn));
                    }
                }
            }
            ImportCategory::Same => {
                // 静默跳过
            }
        }
    }

    (to_create, to_update)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_json_never_contains_secrets() {
        // 创建带有敏感凭据的连接
        let params = SshParams {
            host: "10.0.0.1".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Password {
                password: "SuperSecretPassword123!".to_string(),
            },
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
        };
        let mut conn = StoredConnection::new_ssh("secret-server".to_string(), params, None);
        conn.env = Some("prod".to_string());
        conn.tags = Some(vec!["k8s".to_string(), "prod".to_string()]);

        let workspaces = vec![];
        let item = build_export_item(&conn, &workspaces, false);
        let json_text = generate_export_content(vec![item], ExportFormat::Json, false).unwrap();

        // 断言导出的 JSON 可正常 parse
        let parsed: serde_json::Value = serde_json::from_str(&json_text).expect("Valid JSON");

        // 强断言：任何层级严禁包含 password/passphrase/secret 等凭据键
        fn assert_no_secret_keys(val: &serde_json::Value) {
            match val {
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        let lk = k.to_lowercase();
                        assert!(
                            !lk.contains("password")
                                && !lk.contains("passphrase")
                                && !lk.contains("secret")
                                && !lk.contains("private_key"),
                            "安全不变量被破坏！发现敏感键: {}",
                            k
                        );
                        assert_no_secret_keys(v);
                    }
                }
                serde_json::Value::Array(arr) => {
                    for item in arr {
                        assert_no_secret_keys(item);
                    }
                }
                serde_json::Value::String(s) => {
                    assert!(
                        !s.contains("SuperSecretPassword123!"),
                        "明文密码泄漏到导出内容中！"
                    );
                }
                _ => {}
            }
        }

        assert_no_secret_keys(&parsed);
    }

    #[test]
    fn test_export_json_includes_password_when_requested() {
        let params = SshParams {
            host: "10.0.0.1".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Password {
                password: "SuperSecretPassword123!".to_string(),
            },
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
        };
        let conn = StoredConnection::new_ssh("secret-server".to_string(), params, None);
        let workspaces = vec![];
        let item = build_export_item(&conn, &workspaces, true);
        assert_eq!(item.password.as_deref(), Some("SuperSecretPassword123!"));
        let json_text = generate_export_content(vec![item], ExportFormat::Json, true).unwrap();
        assert!(json_text.contains("SuperSecretPassword123!"));
        assert!(json_text.contains("\"masked\": false"));
    }

    #[test]
    fn test_import_preflight_and_strategies() {
        // 构造本地已有连接
        let p1 = SshParams {
            host: "10.196.2.55".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Agent,
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
        };
        let mut local_conflict = StoredConnection::new_ssh("旧名称".to_string(), p1, None);
        local_conflict.id = Some(101);
        local_conflict.env = Some("dev".to_string());

        let p2 = SshParams {
            host: "10.196.5.29".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Agent,
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
        };
        let mut local_same = StoredConnection::new_ssh("4楼测试机".to_string(), p2, None);
        local_same.id = Some(102);
        local_same.env = Some("test".to_string());
        local_same.tags = Some(vec!["个人".to_string()]);

        let local_connections = vec![local_conflict, local_same];

        // 解析示例文件（包含 1 新增、1 冲突、1 相同）
        let imported = parse_import_payload(sample_import_json()).unwrap();
        assert_eq!(imported.len(), 3);

        let report = preflight_import(imported, &local_connections);
        assert_eq!(report.new_count, 1);
        assert_eq!(report.conflict_count, 1);
        assert_eq!(report.same_count, 1);

        // 策略 1：保留本地
        let (create_1, update_1) = plan_import_execution(&report, MergeStrategy::KeepLocal, &[]);
        assert_eq!(create_1.len(), 1);
        assert_eq!(update_1.len(), 0);

        // 策略 2：覆盖本地
        let (create_2, update_2) = plan_import_execution(&report, MergeStrategy::Overwrite, &[]);
        assert_eq!(create_2.len(), 1);
        assert_eq!(update_2.len(), 1);
        assert_eq!(update_2[0].0, 101);
        assert_eq!(update_2[0].1.name, "k8s-master-1（HA 改造后改名）");
    }
}
