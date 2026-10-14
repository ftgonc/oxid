use std::collections::HashSet;

use oxid::dag::resource_graph::{build_resource_dag, to_dot, ResourceGraph};
use oxid::executor::engine::{build_provider_map, build_variable_defaults};
use oxid::hcl::parse_directory;

// ─── Helper ──────────────────────────────────────────────────────────────────

/// Parse `main.tf` and return the graph's edges as (from, to) addresses.
fn edges(tf: &str) -> (ResourceGraph, HashSet<(String, String)>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.tf"), tf).unwrap();
    let ws = parse_directory(dir.path()).unwrap();
    let (graph, _) =
        build_resource_dag(&ws, &build_provider_map(&ws), &build_variable_defaults(&ws)).unwrap();

    let edges = graph
        .edge_indices()
        .map(|e| {
            let (from, to) = graph.edge_endpoints(e).unwrap();
            (
                graph[from].address().to_string(),
                graph[to].address().to_string(),
            )
        })
        .collect();
    (graph, edges)
}

fn has_edge(edges: &HashSet<(String, String)>, from: &str, to: &str) -> bool {
    edges.contains(&(from.to_string(), to.to_string()))
}

// ─── References through locals ───────────────────────────────────────────────

#[test]
fn test_local_reference_creates_edge_to_data_source() {
    let (_, edges) = edges(
        r#"
data "null_data_source" "network" {}

locals {
  network_id = data.null_data_source.network.id
}

resource "null_resource" "zone" {
  triggers = {
    network = local.network_id
  }
}

resource "null_resource" "direct" {
  name = local.network_id
}
"#,
    );
    assert!(
        has_edge(
            &edges,
            "data.null_data_source.network",
            "null_resource.direct"
        ),
        "{:?}",
        edges
    );
    assert!(
        has_edge(
            &edges,
            "data.null_data_source.network",
            "null_resource.zone"
        ),
        "nested block / map attribute referencing a local: {:?}",
        edges
    );
}

#[test]
fn test_chained_locals_resolve_transitively() {
    let (_, edges) = edges(
        r#"
resource "null_resource" "vcn" {}

locals {
  vcn_id  = null_resource.vcn.id
  by_name = { main = local.vcn_id }
  ids     = [for k, v in local.by_name : v]
}

resource "null_resource" "subnet" {
  vcn = local.ids[0]
}
"#,
    );
    assert!(
        has_edge(&edges, "null_resource.vcn", "null_resource.subnet"),
        "{:?}",
        edges
    );
}

#[test]
fn test_for_each_reference_creates_edge() {
    // for_each = local.x where the local resolves to a resource: the
    // for_each expression is a dependency, not only the attributes
    let (_, edges) = edges(
        r#"
resource "null_resource" "endpoints" {}

locals {
  endpoints = null_resource.endpoints.triggers
}

resource "null_resource" "rules" {
  for_each = local.endpoints
}
"#,
    );
    assert!(
        has_edge(&edges, "null_resource.endpoints", "null_resource.rules"),
        "{:?}",
        edges
    );
}

#[test]
fn test_output_referencing_local_depends_on_resource() {
    let (_, edges) = edges(
        r#"
resource "null_resource" "lb" {}

locals {
  lb_id = null_resource.lb.id
}

output "lb" {
  value = local.lb_id
}
"#,
    );
    assert!(has_edge(&edges, "null_resource.lb", "lb"), "{:?}", edges);
}

#[test]
fn test_cyclic_locals_do_not_hang_or_add_edges() {
    let (_, edges) = edges(
        r#"
locals {
  a = local.b
  b = local.a
}

resource "null_resource" "x" {
  name = local.a
}
"#,
    );
    assert!(edges.is_empty(), "{:?}", edges);
}

#[test]
fn test_var_references_still_ignored() {
    let (_, edges) = edges(
        r#"
variable "name" {
  default = "x"
}

locals {
  name = var.name
}

resource "null_resource" "x" {
  name = local.name
}
"#,
    );
    assert!(edges.is_empty(), "{:?}", edges);
}

// ─── DOT output ──────────────────────────────────────────────────────────────

#[test]
fn test_dot_data_source_label_has_single_prefix() {
    let (graph, _) = edges(r#"data "null_data_source" "network" {}"#);
    let dot = to_dot(&graph);
    assert!(
        dot.contains("label=\"data.null_data_source.network\\n"),
        "{}",
        dot
    );
    assert!(!dot.contains("data.data."), "{}", dot);
}
