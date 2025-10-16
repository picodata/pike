use picotest::*;

// Перед сценарием пытаемся удалить плагин, если он уже есть; ошибки игнорируем
fn best_effort_cleanup(cluster: &Cluster, name: &str, version: &str) {
    let _ = cluster.run_query(format!(r#"ALTER PLUGIN "{name}" {version} DISABLE;"#));
    let _ = cluster.run_query(format!(r#"DROP PLUGIN "{name}" {version} WITH DATA;"#));
}

#[picotest]
fn test_smoke_plugin_lifecycle() {
    let plugin_name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");

    best_effort_cleanup(cluster, plugin_name, version);

    // 1) CREATE PLUGIN
    cluster.run_query(format!(r#"CREATE PLUGIN "{plugin_name}" {version};"#))
        .unwrap_or_else(|err| panic!("CREATE PLUGIN failed: {err}"));

    // 2) MIGRATE TO
    cluster.run_query(format!(
        r#"ALTER PLUGIN "{plugin_name}" MIGRATE TO {version};"#
    )).unwrap_or_else(|err| panic!("MIGRATE TO failed: {err}"));

    // 3) ENABLE
    cluster.run_query(format!(
        r#"ALTER PLUGIN "{plugin_name}" {version} ENABLE;"#
    )).unwrap_or_else(|err| panic!("ENABLE failed: {err}"));

    let table = cluster
        .run_query("SELECT name, enabled FROM _pico_plugin;")
        .expect("Failed to query _pico_plugin after ENABLE");
    assert!(
        table.contains(plugin_name) && table.contains("true"),
        "Plugin should be enabled. Table:\n{table}"
    );

    // 4) DISABLE
    cluster.run_query(format!(
        r#"ALTER PLUGIN "{plugin_name}" {version} DISABLE;"#
    )).unwrap_or_else(|err| panic!("DISABLE failed: {err}"));

    let table = cluster
        .run_query("SELECT name, enabled FROM _pico_plugin;")
        .expect("Failed to query _pico_plugin after DISABLE");
    assert!(
        table.contains(plugin_name) && table.contains("false"),
        "Plugin should be disabled. Table:\n{table}"
    );

    // 5) DROP WITH DATA
    cluster.run_query(format!(
        r#"DROP PLUGIN "{plugin_name}" {version} WITH DATA;"#
    )).unwrap_or_else(|err| panic!("DROP WITH DATA failed: {err}"));

    // Валидация
    let final_table = cluster
        .run_query("SELECT name FROM _pico_plugin;")
        .expect("Failed to query _pico_plugin after DROP");
    assert!(
        !final_table.contains(plugin_name),
        "Plugin '{plugin_name}' still present after DROP WITH DATA. Table:\n{final_table}"
    );
}
