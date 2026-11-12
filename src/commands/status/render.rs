use super::model::{InstanceStatus, InstancesStatus};

/// A missing optional value renders as a dash.
fn dash(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

/// Renders the status as a table with a summary line
pub(super) fn render_text(status: &InstancesStatus) -> String {
    const HEADERS: [&str; 6] = ["TIER", "REPLICASET", "INSTANCE", "DIR", "PROCESS", "STATE"];

    // formats the cells of a row
    let cells = |instance: &InstanceStatus| {
        [
            dash(instance.tier.as_deref()).to_string(),
            dash(instance.replicaset.as_deref()).to_string(),
            format!(
                "{} {}",
                if instance.raft_leader == Some(true) {
                    "★"
                } else {
                    " "
                },
                dash(instance.name.as_deref())
            ),
            instance.dir.clone(),
            instance.process.to_string(),
            instance
                .current_state
                .map_or_else(|| "-".to_string(), |state| state.to_string()),
        ]
    };

    // A column is as wide as its widest cell, but never narrower than its
    // header
    let mut widths = HEADERS.map(str::len);
    for instance in &status.instances {
        for (width, cell) in widths.iter_mut().zip(cells(instance)) {
            *width = (*width).max(cell.chars().count());
        }
    }

    let mut text = String::new();
    for row in
        std::iter::once(HEADERS.map(str::to_string)).chain(status.instances.iter().map(cells))
    {
        let line = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect::<Vec<_>>()
            .join("  ");
        text.push_str(line.trim_end());
        text.push('\n');
    }

    let running = status
        .instances
        .iter()
        .filter(|instance| instance.is_running())
        .count();
    let not_started = status
        .instances
        .iter()
        .filter(|instance| instance.not_started())
        .count();

    let mut summary = format!(
        "{}/{} instances running, {not_started} not started",
        running,
        status.instances.len()
    );
    if !status.unexpected.is_empty() {
        summary = format!(
            "{summary}, {} not described in topology.toml: {}",
            status.unexpected.len(),
            status.unexpected.join(", ")
        );
    }

    text.push('\n');
    text.push_str(&summary);
    text.push_str("\n★ — raft leader");
    text
}
