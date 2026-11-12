use super::collector::{extract_expected_instances, parse_instance_n, DirectoryInstance};
use super::merge::merge;
use super::model::{InstanceStatus, InstancesStatus, ProcessState};
use super::render::render_text;
use crate::commands::lib::instance_info::{ClusterInstance, InstanceState};
use crate::commands::run::{Tier, Topology};
use std::collections::HashMap;

fn make_topology(tiers: &[(&str, u8, u8)]) -> Topology {
    Topology {
        tiers: tiers
            .iter()
            .map(|(name, replicasets, replication_factor)| {
                (
                    (*name).to_string(),
                    Tier {
                        replicasets: *replicasets,
                        replication_factor: *replication_factor,
                    },
                )
            })
            .collect(),
        ..Default::default()
    }
}

fn cluster_row(name: &str, tier: &str, replicaset: &str, is_leader: bool) -> ClusterInstance {
    ClusterInstance {
        name: name.to_string(),
        tier: tier.to_string(),
        replicaset_name: replicaset.to_string(),
        current_state: InstanceState::Online,
        is_leader,
    }
}

#[test]
fn extract_expected_instances_order() {
    // declare two tiers (router after storage) but tiers alphabetically sorted
    let topology = make_topology(&[("storage", 1, 2), ("router", 2, 1)]);

    assert_eq!(
        extract_expected_instances(&topology),
        vec![
            (1, "router".to_string()),
            (2, "router".to_string()),
            (3, "storage".to_string()),
            (4, "storage".to_string()),
        ]
    );
}

#[test]
fn merge_enriches_from_cluster_by_name() {
    let cluster = vec![cluster_row("default_1_1", "default", "default_1", true)];
    let topology = make_topology(&[("default", 1, 1)]);
    let found_on_disk = [DirectoryInstance {
        n: 1,
        process: ProcessState::Running,
        instance_name: Some("default_1_1".to_string()),
    }];

    let status = merge(
        &extract_expected_instances(&topology),
        &found_on_disk,
        Some(cluster.as_slice()),
    );

    assert_eq!(status.unexpected, Vec::<String>::new());
    let instance = &status.instances[0];
    assert_eq!(instance.tier.as_deref(), Some("default"));
    assert_eq!(instance.replicaset.as_deref(), Some("default_1"));
    assert_eq!(instance.name.as_deref(), Some("default_1_1"));
    assert_eq!(instance.raft_leader, Some(true));
    assert_eq!(instance.dir, "i1");
    assert_eq!(instance.process, ProcessState::Running);
    assert_eq!(instance.current_state, Some(InstanceState::Online));
}

#[test]
fn topology_instances_without_directory_considered_not_started() {
    // tier with 2x2 instances
    let topology_tier = "default";
    let topology = make_topology(&[(topology_tier, 2, 2)]);
    // on disk found only 1 instance
    let instances_on_disk = [DirectoryInstance {
        n: 1,
        process: ProcessState::Running,
        instance_name: Some(format!("{topology_tier}_1_1")),
    }];

    let status = merge(
        &extract_expected_instances(&topology),
        &instances_on_disk,
        None,
    );

    let mut running = Vec::new();
    let mut not_started = Vec::new();
    let mut stopped = Vec::new();

    for ins_output in &status.instances {
        let ins_dir = ins_output.dir.as_str();
        match ins_output.process {
            ProcessState::Running => {
                running.push(ins_dir);
            }
            ProcessState::NotStarted => {
                not_started.push(ins_dir);
            }
            ProcessState::Stopped => {
                stopped.push(ins_dir);
            }
        }
    }

    assert_eq!(running, vec!["i1"]);
    assert_eq!(not_started, vec!["i2", "i3", "i4"]);
    assert_eq!(stopped, [] as [&str; 0]);

    // every not-started instance keeps the expected tier from topology.toml;
    // with no cluster data there is no replicaset and the leader is unknown
    for instance in &status.instances {
        if !instance.not_started() {
            continue;
        }
        assert_eq!(
            (
                instance.tier.as_deref(),
                instance.replicaset.as_deref(),
                instance.raft_leader
            ),
            (Some(topology_tier), None, None),
            "wrong fields of {}",
            instance.dir
        );
    }
}

#[test]
fn extra_directories_considered_unexpected() {
    // topology.toml with 1x1 instances
    let topology = make_topology(&[("default", 1, 1)]);
    // cluster data with 2x1 instances
    let cluster = vec![
        cluster_row("default_1_1", "default", "default_1", true),
        cluster_row("default_3_1", "default", "default_3", false),
    ];
    // 2x1 instances on disk
    let instances_on_disk = [
        DirectoryInstance {
            n: 1,
            process: ProcessState::Running,
            instance_name: Some("default_1_1".to_string()),
        },
        DirectoryInstance {
            n: 5,
            process: ProcessState::Running,
            instance_name: Some("default_3_1".to_string()),
        },
    ];

    let status_output = merge(
        &extract_expected_instances(&topology),
        &instances_on_disk,
        Some(cluster.as_slice()),
    );

    assert_eq!(status_output.unexpected, vec!["i5"]);
    // an extra instance
    let extra = status_output
        .instances
        .iter()
        .find(|instance| instance.dir == "i5")
        .unwrap();
    assert_eq!(extra.tier.as_deref(), Some("default"));
    assert_eq!(extra.replicaset.as_deref(), Some("default_3"));
    assert_eq!(extra.raft_leader, Some(false));
}

#[test]
fn extra_directory_without_cluster_data() {
    let status = merge(
        &extract_expected_instances(&make_topology(&[("default", 1, 1)])),
        &[DirectoryInstance {
            n: 5,
            process: ProcessState::Stopped,
            instance_name: Some("default_3_1".to_string()),
        }],
        None,
    );

    let extra = &status.instances[1];
    assert_eq!(status.unexpected, vec!["i5"]);
    assert_eq!(
        (
            extra.tier.as_ref(),
            extra.replicaset.as_ref(),
            extra.current_state.as_ref(),
            extra.raft_leader
        ),
        (None, None, None, None)
    );
    // The name survives via the symlink even without cluster data.
    assert_eq!(extra.name.as_deref(), Some("default_3_1"));
}

#[test]
fn merge_without_symlink() {
    let cluster = vec![cluster_row("default_1_1", "default", "default_1", true)];
    let status = merge(
        &extract_expected_instances(&make_topology(&[("default", 1, 1)])),
        &[DirectoryInstance {
            n: 1,
            process: ProcessState::Running,
            instance_name: None,
        }],
        Some(cluster.as_slice()),
    );

    let instance = &status.instances[0];
    assert_eq!(instance.name, None);
    // Without a name there is nothing to join the cluster row by
    assert_eq!(instance.replicaset, None);
    assert_eq!(instance.current_state, None);
    // The leader is known by cluster data
    assert_eq!(instance.raft_leader, Some(false));
    // The tier falls back to the expected one
    assert_eq!(instance.tier.as_deref(), Some("default"));
}

#[test]
fn raft_leader_unknown() {
    // Elections in progress: no row is marked as the leader
    let cluster = vec![cluster_row("default_1_1", "default", "default_1", false)];
    let status = merge(
        &extract_expected_instances(&make_topology(&[("default", 1, 1)])),
        &[DirectoryInstance {
            n: 1,
            process: ProcessState::Running,
            instance_name: Some("default_1_1".to_string()),
        }],
        Some(cluster.as_slice()),
    );
    assert_eq!(status.instances[0].raft_leader, None);
}

#[test]
fn raft_leader_single_mark() {
    let cluster = vec![
        cluster_row("default_1_1", "default", "default_1", false),
        cluster_row("default_1_2", "default", "default_1", true),
    ];
    let status = merge(
        &extract_expected_instances(&make_topology(&[("default", 1, 2)])),
        &[
            DirectoryInstance {
                n: 1,
                process: ProcessState::Running,
                instance_name: Some("default_1_1".to_string()),
            },
            DirectoryInstance {
                n: 2,
                process: ProcessState::Running,
                instance_name: Some("default_1_2".to_string()),
            },
            DirectoryInstance {
                n: 3,
                process: ProcessState::Running,
                instance_name: None,
            },
        ],
        Some(cluster.as_slice()),
    );

    let by_dir: HashMap<&str, Option<bool>> = status
        .instances
        .iter()
        .map(|instance| (instance.dir.as_str(), instance.raft_leader))
        .collect();
    assert_eq!(by_dir["i1"], Some(false));
    assert_eq!(by_dir["i2"], Some(true));
    assert_eq!(by_dir["i3"], Some(false));
}

#[test]
fn merge_sorts() {
    let cluster = vec![
        cluster_row("a_z", "a", "z", false),
        cluster_row("a_a", "a", "a", false),
    ];
    // tier "a": i1..i4
    // tier "b": i5
    // i9 is an extra directory
    let status = merge(
        &extract_expected_instances(&make_topology(&[("a", 2, 2), ("b", 1, 1)])),
        &[
            DirectoryInstance {
                n: 1,
                process: ProcessState::Running,
                instance_name: Some("a_none".to_string()),
            },
            DirectoryInstance {
                n: 2,
                process: ProcessState::Running,
                instance_name: Some("a_z".to_string()),
            },
            DirectoryInstance {
                n: 3,
                process: ProcessState::Running,
                instance_name: Some("a_a".to_string()),
            },
            DirectoryInstance {
                n: 9,
                process: ProcessState::Stopped,
                instance_name: None,
            },
        ],
        Some(cluster.as_slice()),
    );

    let orders: Vec<(Option<&str>, Option<&str>, &str)> = status
        .instances
        .iter()
        .map(|instance| {
            (
                instance.tier.as_deref(),
                instance.replicaset.as_deref(),
                instance.dir.as_str(),
            )
        })
        .collect();

    assert_eq!(
        orders,
        vec![
            (Some("a"), Some("a"), "i3"), // "a" replicaset goes before "z" replicaset
            (Some("a"), Some("z"), "i2"),
            (Some("a"), None, "i1"), // no replicaset, missing in the cluster data
            (Some("a"), None, "i4"), // no replicaset, not-started
            (Some("b"), None, "i5"), // "a" tier goes before "b" tier
            (None, None, "i9"),      // extra directory, has no tier
        ]
    );
    assert_eq!(status.unexpected, vec!["i9"]);
}

#[test]
fn parse_instance_dir() {
    assert_eq!(parse_instance_n("i1"), Some(1));
    assert_eq!(parse_instance_n("i12"), Some(12));
    assert_eq!(parse_instance_n("default_1_1"), None);
    assert_eq!(parse_instance_n("i"), None);
    assert_eq!(parse_instance_n("ix"), None);
}

#[test]
fn text_output_lines() {
    let status = InstancesStatus {
        instances: vec![
            InstanceStatus {
                tier: Some("default".to_string()),
                replicaset: Some("default_1".to_string()),
                name: Some("default_1_1".to_string()),
                raft_leader: Some(true),
                dir: "i1".to_string(),
                process: ProcessState::Running,
                current_state: Some(InstanceState::Online),
            },
            InstanceStatus {
                tier: Some("non_default".to_string()),
                replicaset: None,
                name: None,
                raft_leader: Some(false),
                dir: "i2".to_string(),
                process: ProcessState::NotStarted,
                current_state: None,
            },
        ],
        unexpected: vec!["i3".to_string()],
    };

    let text = render_text(&status);

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 6);
    assert_eq!(
        lines[0],
        "TIER         REPLICASET  INSTANCE       DIR  PROCESS      STATE"
    );
    assert_eq!(
        lines[1],
        "default      default_1   ★ default_1_1  i1   running      Online"
    );
    assert_eq!(
        lines[2],
        "non_default  -             -            i2   not_started  -"
    );
    assert_eq!(lines[3], "");
    assert_eq!(
        lines[4],
        "1/2 instances running, 1 not started, 1 not described in topology.toml: i3"
    );
    assert_eq!(lines[5], "★ — raft leader");
}

#[test]
fn json_output() {
    let status = InstancesStatus {
        instances: vec![
            InstanceStatus {
                tier: Some("default".to_string()),
                replicaset: Some("default_1".to_string()),
                name: Some("default_1_1".to_string()),
                raft_leader: Some(true),
                dir: "i1".to_string(),
                process: ProcessState::Running,
                current_state: Some(InstanceState::Online),
            },
            InstanceStatus {
                tier: Some("non_default".to_string()),
                replicaset: None,
                name: None,
                raft_leader: Some(false),
                dir: "i2".to_string(),
                process: ProcessState::NotStarted,
                current_state: None,
            },
        ],
        unexpected: vec!["i3".to_string()],
    };

    let json = serde_json::to_string_pretty(&status).unwrap();

    assert_eq!(
        json,
        r#"{
  "instances": [
    {
      "tier": "default",
      "replicaset": "default_1",
      "name": "default_1_1",
      "raft_leader": true,
      "dir": "i1",
      "process": "running",
      "current_state": "Online"
    },
    {
      "tier": "non_default",
      "replicaset": null,
      "name": null,
      "raft_leader": false,
      "dir": "i2",
      "process": "not_started",
      "current_state": null
    }
  ],
  "unexpected": [
    "i3"
  ]
}"#
    );
}
