use super::collector::DirectoryInstance;
use super::model::{InstanceStatus, InstancesStatus};
use crate::commands::lib::instance_info::ClusterInstance;
use crate::commands::run::PicodataInstance;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

/// Merges the expected instances with the found ones and enriches the result
/// with cluster data, joined by the instance name from the symlink.
pub(super) fn merge(
    expected_by_topology: &[(u16, String)],
    found_on_disk: &[DirectoryInstance],
    cluster_data: Option<&[ClusterInstance]>,
) -> InstancesStatus {
    // index instances from cluster info by name for fast access
    let rows_by_name: HashMap<&str, &ClusterInstance> = cluster_data
        .map(|rows| rows.iter().map(|row| (row.name.as_str(), row)).collect())
        .unwrap_or_default();
    // leader flag is true on at most one row (raft ids are unique)
    // all-false means the leader is unknown
    let leader_known = cluster_data.is_some_and(|rows| rows.iter().any(|row| row.is_leader));

    // collect and sort unique numbers of instances from expected and found
    let mut ns = BTreeSet::from_iter(expected_by_topology.iter().map(|(n, _)| *n));
    ns.extend(found_on_disk.iter().map(|instance| instance.n));

    // directories of found instances missing from topology.toml
    let unexpected: Vec<String> = ns
        .iter()
        .filter(|n| {
            !expected_by_topology
                .iter()
                .any(|(expected_n, _)| expected_n == *n)
        })
        .map(|n| PicodataInstance::make_data_dir_name(*n))
        .collect();

    let mut output_rows = Vec::with_capacity(ns.len());
    for n in ns {
        let expected_tier = expected_by_topology
            .iter()
            .find(|(expected_n, _)| *expected_n == n)
            .map(|(_, tier)| tier.as_str());
        let found = found_on_disk.iter().find(|instance| instance.n == n);
        let row = found
            .and_then(|instance| instance.instance_name.as_deref())
            .and_then(|name| rows_by_name.get(name).copied());

        output_rows.push(InstanceStatus::new(
            n,
            found,
            expected_tier,
            row,
            leader_known,
        ));
    }

    // sort rows by tier and then replicaset (alphabetical order, null goes last)
    // the sort is stable and the rows above are built in ascending-N order
    // (`ns` is a BTreeSet), so equal instance keys keep the by-N order
    output_rows.sort_by(|a, b| {
        cmp_opt(a.tier.as_deref(), b.tier.as_deref())
            .then_with(|| cmp_opt(a.replicaset.as_deref(), b.replicaset.as_deref()))
    });

    InstancesStatus {
        instances: output_rows,
        unexpected,
    }
}

/// `None` sorts last: without a tier (or replicaset) an instance goes after
/// all the named ones.
fn cmp_opt(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
