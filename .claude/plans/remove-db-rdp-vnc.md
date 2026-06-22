# 移除数据库、RDP、VNC连接功能重构计划

## 目标
移除项目中的数据库（MySQL、PostgreSQL、SQLite、MSSQL、Oracle、ClickHouse、DuckDB）和远程桌面（RDP、VNC）连接相关功能及代码。

## 范围
### 需要移除的 crate
1. `crates/db` - 数据库核心逻辑
2. `crates/db_view` - 数据库视图组件
3. `crates/remote_desktop` - 远程桌面核心逻辑
4. `crates/remote_desktop_view` - 远程桌面视图组件

### 需要修改的 crate
1. `main/` - 主应用入口，移除数据库和远程桌面集成
2. `crates/core/` - 核心存储模型，移除相关类型定义
3. `crates/extension-runtime/` - 扩展运行时，移除数据库相关扩展支持
4. `crates/ui/` - UI组件库，移除相关图标和组件

## 分阶段实施计划

### 阶段一：移除 RDP/VNC 远程桌面功能

#### 步骤 1.1：从主应用移除远程桌面集成
- 修改 `main/Cargo.toml`：移除 `remote_desktop` 和 `remote_desktop_view` 依赖
- 修改 `main/src/onetcli_app.rs`：
  - 移除 `remote_desktop_view::init(cx);` 调用
  - 移除 `remote_desktop_view::refresh_keybindings(cx);` 调用
- 修改 `main/src/home_tab.rs`：
  - 移除 `RemoteDesktopFormWindow` 和 `RemoteDesktopFormWindowConfig` 导入
  - 移除 `show_remote_desktop_form` 方法
  - 移除编辑连接中的 RDP/VNC 分支
  - 移除连接卡片中的 RDP/VNC 图标
  - 移除搜索匹配中的 RDP/VNC 分支
- 修改 `main/src/new_connection/connection_kind.rs`：
  - 移除 `Rdp` 和 `Vnc` 变体
  - 移除相关图标和标签
- 修改 `main/src/new_connection/mod.rs`：
  - 移除 `remote_desktop_form` 模块

#### 步骤 1.2：从核心存储模型移除远程桌面类型
- 修改 `crates/core/src/storage/models.rs`：
  - 移除 `ConnectionType::Rdp` 和 `ConnectionType::Vnc`
  - 移除 `RemoteDesktopProtocol` 枚举
  - 移除 `RemoteDesktopParams` 结构体
  - 移除 `StoredConnection::new_remote_desktop` 方法
  - 移除 `to_remote_desktop_params` 方法
  - 更新 `ConnectionType::all()` 和相关方法

#### 步骤 1.3：移除远程桌面 crate
- 删除 `crates/remote_desktop` 目录
- 删除 `crates/remote_desktop_view` 目录
- 从根 `Cargo.toml` 中移除相关成员和依赖

#### 步骤 1.4：清理其他依赖
- 修改 `crates/extension-runtime/Cargo.toml`：移除远程桌面相关依赖（如果有）
- 检查并清理任何其他引用远程桌面的代码

### 阶段二：移除数据库功能

#### 步骤 2.1：从主应用移除数据库集成
- 修改 `main/Cargo.toml`：
  - 移除 `db` 和 `db_view` 依赖
  - 移除相关 feature flags（`builtin-duckdb`、`compare`）
- 修改 `main/src/main.rs`：
  - 移除 `db::ipc::DriverAssetSource` 和相关代码
  - 简化 `AppAssets` 结构
- 修改 `main/src/onetcli_app.rs`：
  - 移除 `db_view::search_shortcut::init(cx);`
  - 移除 `db_view::sql_editor_view::init(cx);`
  - 移除 `db_view::chatdb::agents::init(cx);`
  - 移除 `db::init_cache(cx);`
  - 移除 `db_view::init_ask_ai_notifier(cx);`
  - 移除 `GlobalDbState` 相关代码
  - 移除 `db_view::search_shortcut::refresh_keybindings(cx);`
  - 移除 `db_view::sql_editor_view::refresh_keybindings(cx);`
- 修改 `main/src/home_tab.rs`：
  - 移除 `db_view::connection_form_window` 导入
  - 移除 `DatabaseTabView` 和 `ChatPanel` 导入
  - 移除数据库相关的连接打开逻辑
  - 移除数据库连接表单显示逻辑
- 修改 `main/src/home/home_tabs.rs`：
  - 移除数据库标签页相关代码
- 修改 `main/src/new_connection/`：
  - 移除数据库连接表单相关代码

#### 步骤 2.2：从核心存储模型移除数据库类型
- 修改 `crates/core/src/storage/models.rs`：
  - 移除 `DatabaseType` 枚举（如果完全不需要）
  - 或保留但移除所有具体数据库类型变体
  - 移除 `DbConnectionConfig` 结构体（如果不需要）
  - 移除相关方法和测试

#### 步骤 2.3：移除数据库 crate
- 删除 `crates/db` 目录
- 删除 `crates/db_view` 目录
- 从根 `Cargo.toml` 中移除相关成员和依赖

#### 步骤 2.4：清理扩展运行时
- 修改 `crates/extension-runtime/src/`：
  - 移除数据库相关的扩展支持代码
  - 移除 `extension_db_gateway` 等模块
  - 更新扩展注册和动作处理

### 阶段三：清理和验证

#### 步骤 3.1：清理配置和依赖
- 更新根 `Cargo.toml`：移除所有数据库相关的 workspace 依赖
- 清理 `Cargo.lock`：运行 `cargo update` 重新生成
- 检查并清理任何其他引用已删除 crate 的代码

#### 步骤 3.2：验证编译
- 运行 `cargo check` 确保所有代码编译通过
- 运行 `cargo clippy -- --deny warnings` 确保没有警告
- 运行 `cargo fmt --check` 确保代码格式正确

#### 步骤 3.3：运行测试
- 运行 `cargo test --all` 确保所有测试通过
- 检查是否有遗漏的测试需要清理

#### 步骤 3.4：文档更新
- 更新 `CLAUDE.md` 中的项目概述和工作区结构
- 更新 `README.md`（如果存在）
- 清理任何其他文档中的相关引用

## 风险和注意事项

### 数据迁移
- 移除功能后，用户现有的数据库连接配置将无法使用
- 需要考虑是否提供数据迁移工具或保留配置文件兼容性

### 扩展兼容性
- 如果有扩展依赖于数据库功能，可能需要更新或移除这些扩展
- 需要检查扩展 API 中是否有数据库相关的接口

### 测试覆盖
- 确保移除功能后，剩余功能的测试仍然通过
- 可能需要补充一些集成测试

## 验证方式

1. **编译验证**：`cargo check` 和 `cargo clippy`
2. **测试验证**：`cargo test --all`
3. **功能验证**：启动应用，验证剩余功能正常工作
4. **依赖验证**：检查 `cargo tree` 确保没有循环依赖或未使用的依赖

## 时间估计

- 阶段一（RDP/VNC）：2-3 小时
- 阶段二（数据库）：4-6 小时
- 阶段三（清理验证）：2-3 小时
- 总计：8-12 小时
