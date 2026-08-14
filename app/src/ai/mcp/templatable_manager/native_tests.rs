//! Regression coverage for the built-in Factory MCP reconnect race
//! (APP-5381): a credential-rotation respawn racing a concurrent
//! `reconnect_server` call must not abort the valid, in-flight replacement
//! spawn, and ephemeral (non-persisted) installations must be reconnectable
//! without ever appearing in user-managed MCP settings.
//!
//! These tests exercise `TemplatableMCPServerManager` directly against a
//! real, minimal, in-process streamable-HTTP MCP server, bypassing
//! `TemplatableMCPServerManager::new` (and its many singleton dependencies)
//! since only the manager's own spawn/reconnect bookkeeping is under test.

use std::collections::HashMap;
use std::net::SocketAddr;

use rmcp::ServerHandler;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::sync::oneshot;
use uuid::Uuid;
use warpui::{App, ModelHandle};

use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::ai::mcp::{JsonTemplate, TemplatableMCPServer, TemplatableMCPServerInstallation};

/// A minimal MCP server with no tools/resources; all `ServerHandler` methods use their
/// protocol-compliant defaults, which is all the manager's spawn/reconnect path needs.
#[derive(Clone, Default)]
struct FakeMcpServer;

impl ServerHandler for FakeMcpServer {}

/// Binds a real streamable-HTTP MCP server to an ephemeral localhost port, but holds off
/// serving connections until the returned sender fires.
///
/// Binding (and so learning the port) happens immediately; the TCP backlog accepts the
/// manager's connection attempt even before `axum::serve` starts its accept loop, so a caller
/// that spawns against this address observes a genuinely *pending* connection - not a race
/// against however fast a real handshake happens to complete - until the test releases it.
async fn start_deferred_fake_mcp_server(app: &App) -> (SocketAddr, oneshot::Sender<()>) {
    let (addr_tx, addr_rx) = oneshot::channel::<SocketAddr>();
    let (release_tx, release_rx) = oneshot::channel::<()>();

    app.background_executor()
        .spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:0").await else {
                return;
            };
            let Ok(addr) = listener.local_addr() else {
                return;
            };
            let _ = addr_tx.send(addr);

            if release_rx.await.is_err() {
                return;
            }

            let service: StreamableHttpService<FakeMcpServer, LocalSessionManager> =
                StreamableHttpService::new(
                    || Ok(FakeMcpServer),
                    Default::default(),
                    StreamableHttpServerConfig::default(),
                );
            let router = axum::Router::new().nest_service("/mcp", service);
            let _ = axum::serve(listener, router).await;
        })
        .detach();

    let addr = addr_rx
        .await
        .expect("fake MCP server should report its bound address");
    (addr, release_tx)
}

/// Builds an ephemeral-style `TemplatableMCPServerInstallation` (mirroring the shape of the
/// built-in Factory MCP installation) pointed at a local test server.
fn ephemeral_installation(
    installation_uuid: Uuid,
    name: &str,
    addr: SocketAddr,
) -> TemplatableMCPServerInstallation {
    let mut root = serde_json::Map::new();
    root.insert(
        name.to_string(),
        serde_json::json!({ "url": format!("http://{addr}/mcp") }),
    );
    let template_json = serde_json::Value::Object(root).to_string();

    let templatable_mcp_server = TemplatableMCPServer {
        uuid: Uuid::new_v4(),
        name: name.to_string(),
        description: None,
        template: JsonTemplate {
            json: template_json,
            variables: Vec::new(),
        },
        version: 0,
        gallery_data: None,
    };

    TemplatableMCPServerInstallation::new(installation_uuid, templatable_mcp_server, HashMap::new())
}

/// Registers the minimal singletons `spawn_server_impl` unconditionally touches
/// (`AppExecutionMode`, `GlobalResourceHandlesProvider`) plus `LogManager`, without pulling in
/// the manager's full dependency graph (`FileBasedMCPManager`, `CloudModel`, `AuthStateProvider`,
/// ...), which these tests never exercise since they call manager methods directly instead of
/// going through `TemplatableMCPServerManager::new`/`sync_builtin_servers`.
fn setup_app(app: &mut App) -> ModelHandle<TemplatableMCPServerManager> {
    crate::test_util::settings::initialize_history_persistence_for_tests(app);
    app.add_singleton_model(|_| simple_logger::manager::LogManager::new());
    app.add_model(|_| TemplatableMCPServerManager::default())
}

/// Joins a caller onto whatever spawn (pending or otherwise) is in flight for `installation_uuid`
/// and awaits its result, without an arbitrary poll loop.
async fn join_reconnect(
    manager: &ModelHandle<TemplatableMCPServerManager>,
    app: &mut App,
    installation_uuid: Uuid,
) -> Result<rmcp::Peer<rmcp::RoleClient>, String> {
    let (tx, rx) = oneshot::channel();
    manager.update(app, |m, ctx| m.reconnect_server(installation_uuid, tx, ctx));
    rx.await
        .expect("reconnect result channel should not be dropped")
}

/// Covers the core race: a credential-rotation respawn (`shutdown_server` +
/// `spawn_ephemeral_server`, as `sync_builtin_servers` does) leaves a spawn pending, and two
/// concurrent `reconnect_server` callers must join it - not abort it - and both receive the
/// replacement peer once it completes. Also asserts the reconnected installation remains a
/// tracked built-in and stays out of user-managed MCP settings.
#[test]
fn reconnect_joins_pending_respawn_and_notifies_all_waiters_on_success() {
    App::test((), |mut app| async move {
        let (addr, release_tx) = start_deferred_fake_mcp_server(&app).await;
        let manager = setup_app(&mut app);

        let installation_uuid = Uuid::new_v4();
        let installation = ephemeral_installation(installation_uuid, "fake-factory", addr);

        // Mirror `sync_builtin_servers`'s bookkeeping for a built-in it owns.
        manager.update(&mut app, |m, ctx| {
            m.builtin_server_uuids.insert(installation_uuid);
            // Simulate a credential-rotation respawn: shut down the (nonexistent) old
            // instance, then start the replacement spawn. It stays pending because the
            // fake server is holding off its handshake.
            m.shutdown_server(installation_uuid, ctx);
            m.spawn_ephemeral_server(installation.clone(), ctx);
        });

        manager.read(&app, |m, _| {
            assert!(
                m.spawned_servers.contains_key(&installation_uuid),
                "the replacement spawn should be pending"
            );
            assert!(!m.active_servers.contains_key(&installation_uuid));
        });

        // Two concurrent callers race a reconnect while that spawn is still pending.
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        manager.update(&mut app, |m, ctx| {
            m.reconnect_server(installation_uuid, tx1, ctx)
        });
        manager.update(&mut app, |m, ctx| {
            m.reconnect_server(installation_uuid, tx2, ctx)
        });

        // Neither call should have aborted the pending spawn or started a competing one.
        manager.read(&app, |m, _| {
            assert!(
                m.spawned_servers.contains_key(&installation_uuid),
                "reconnect must not abort a valid pending spawn"
            );
            assert_eq!(
                m.pending_reconnections
                    .get(&installation_uuid)
                    .map(Vec::len),
                Some(2),
                "both callers should be queued as waiters on the same spawn"
            );
        });

        // Let the replacement spawn's handshake complete.
        let _ = release_tx.send(());

        rx1.await
            .expect("waiter channel should not be dropped")
            .expect("reconnect should succeed once the pending spawn completes");
        rx2.await
            .expect("waiter channel should not be dropped")
            .expect("reconnect should succeed once the pending spawn completes");

        manager.read(&app, |m, _| {
            assert!(m.active_servers.contains_key(&installation_uuid));
            assert!(!m.spawned_servers.contains_key(&installation_uuid));
            assert!(!m.pending_reconnections.contains_key(&installation_uuid));
            assert!(
                m.get_active_builtin_servers()
                    .contains_key(&installation_uuid),
                "Factory should remain a tracked, active built-in after the rotation"
            );
            assert!(
                m.get_installed_server(&installation_uuid).is_none(),
                "an ephemeral built-in must never appear as a user-managed installation"
            );
        });
    });
}

/// After an ephemeral peer disconnects with no spawn pending, a reconnect must be able to
/// reconstruct the server from its retained ephemeral installation - the same lookup path a
/// persisted installation uses - even though it was never added to `locally_installed_servers`.
#[test]
fn reconnect_uses_retained_ephemeral_installation_after_peer_closes() {
    App::test((), |mut app| async move {
        let (addr, release_tx) = start_deferred_fake_mcp_server(&app).await;
        let _ = release_tx.send(()); // Serve immediately; this test isn't exercising the race.
        let manager = setup_app(&mut app);

        let installation_uuid = Uuid::new_v4();
        let installation = ephemeral_installation(installation_uuid, "fake-ephemeral", addr);

        manager.update(&mut app, |m, ctx| {
            m.spawn_ephemeral_server(installation.clone(), ctx);
        });

        // Join the in-flight initial spawn to learn when it completes.
        join_reconnect(&manager, &mut app, installation_uuid)
            .await
            .expect("initial spawn should succeed");

        manager.read(&app, |m, _| {
            assert!(m.active_servers.contains_key(&installation_uuid));
            assert!(
                m.get_installed_server(&installation_uuid).is_none(),
                "ephemeral installations must stay out of user-managed MCP settings"
            );
        });

        // Simulate the peer closing (e.g. observed as `TransportClosed`) with no spawn pending.
        manager.update(&mut app, |m, _ctx| {
            m.active_servers.remove(&installation_uuid);
        });

        join_reconnect(&manager, &mut app, installation_uuid)
            .await
            .expect("reconnect should succeed using the retained ephemeral installation");

        manager.read(&app, |m, _| {
            assert!(m.active_servers.contains_key(&installation_uuid));
        });
    });
}

/// Ending an ephemeral server's lifecycle (`shutdown_server`, as the built-in teardown path in
/// `sync_builtin_servers` does) must drop its retained installation - and so the credentials
/// embedded in it, e.g. a bearer token baked into the built-in Factory MCP's headers - and a
/// later reconnect for the same UUID must fail cleanly instead of resurrecting stale state.
#[test]
fn shutdown_removes_retained_ephemeral_installation_and_credentials() {
    App::test((), |mut app| async move {
        let (addr, release_tx) = start_deferred_fake_mcp_server(&app).await;
        let _ = release_tx.send(());
        let manager = setup_app(&mut app);

        let installation_uuid = Uuid::new_v4();
        let installation = ephemeral_installation(installation_uuid, "fake-ephemeral", addr);

        manager.update(&mut app, |m, ctx| {
            m.spawn_ephemeral_server(installation.clone(), ctx);
        });
        join_reconnect(&manager, &mut app, installation_uuid)
            .await
            .expect("initial spawn should succeed");

        manager.read(&app, |m, _| {
            assert!(
                m.reconnectable_ephemeral_installations
                    .contains_key(&installation_uuid),
                "the installation should be retained while the server is running"
            );
        });

        manager.update(&mut app, |m, ctx| {
            m.shutdown_server(installation_uuid, ctx);
        });

        manager.read(&app, |m, _| {
            assert!(
                !m.reconnectable_ephemeral_installations
                    .contains_key(&installation_uuid),
                "shutdown must drop the retained installation (and any embedded credentials)"
            );
            assert!(!m.active_servers.contains_key(&installation_uuid));
            assert!(!m.spawned_servers.contains_key(&installation_uuid));
        });

        // A later reconnect for the same UUID has nothing left to resolve.
        match join_reconnect(&manager, &mut app, installation_uuid).await {
            Err(message) => assert_eq!(message, "Installation not found"),
            Ok(_) => panic!("reconnect should fail once the installation is no longer retained"),
        }
    });
}
