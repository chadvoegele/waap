use std::collections::{BTreeMap, HashMap, HashSet};

use super::TicketMetadata;

/// Validate an immutable candidate graph before publishing a ticket change.
pub(crate) fn validate_dependencies(tickets: &[TicketMetadata]) -> Vec<String> {
    let graph: BTreeMap<&str, &[String]> = tickets
        .iter()
        .map(|ticket| {
            (
                ticket.ticket_id.as_str(),
                ticket.depends_on.as_deref().unwrap_or_default(),
            )
        })
        .collect();
    let mut errors = Vec::new();
    for (id, dependencies) in &graph {
        for dependency in *dependencies {
            if !graph.contains_key(dependency.as_str()) {
                errors.push(format!(
                    "tickets/{id}/ticket.md depends_on {dependency:?} which does not exist"
                ));
            }
        }
    }
    append_cycle_errors(&graph, &mut errors);
    errors
}

fn append_cycle_errors(graph: &BTreeMap<&str, &[String]>, errors: &mut Vec<String>) {
    let mut visited = HashSet::new();
    for &root in graph.keys() {
        if !visited.insert(root) {
            continue;
        }
        let mut path = vec![(root, 0)];
        let mut active = HashMap::from([(root, 0)]);
        while let Some(&(id, next)) = path.last() {
            let dependencies = graph.get(id).copied().unwrap_or_default();
            let Some(dependency) = dependencies.get(next).map(String::as_str) else {
                active.remove(id);
                path.pop();
                continue;
            };
            path.last_mut().expect("nonempty traversal path").1 += 1;
            if let Some(&start) = active.get(dependency) {
                let nodes: Vec<&str> = path[start..].iter().map(|&(id, _)| id).collect();
                errors.push(format!(
                    "dependency cycle detected: {} -> {dependency}",
                    nodes.join(" -> ")
                ));
            } else if visited.insert(dependency) {
                active.insert(dependency, path.len());
                path.push((dependency, 0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: &str, dependencies: &[&str]) -> TicketMetadata {
        TicketMetadata {
            ticket_id: id.into(),
            name: None,
            creation_date: "2026-06-22T12:00:00Z".into(),
            status: "pending".into(),
            depends_on: Some(dependencies.iter().map(|id| (*id).into()).collect()),
        }
    }

    #[test]
    fn reports_missing_dependencies_and_cycles_in_stable_order() {
        let mut tickets = vec![
            ticket("tt-b", &["tt-a"]),
            ticket("tt-a", &["tt-b", "tt-missing"]),
        ];
        let expected = vec![
            "tickets/tt-a/ticket.md depends_on \"tt-missing\" which does not exist",
            "dependency cycle detected: tt-a -> tt-b -> tt-a",
        ];
        assert_eq!(validate_dependencies(&tickets), expected);
        tickets.reverse();
        assert_eq!(validate_dependencies(&tickets), expected);
    }

    #[test]
    fn detects_self_dependency() {
        assert_eq!(
            validate_dependencies(&[ticket("tt-a", &["tt-a"])]),
            ["dependency cycle detected: tt-a -> tt-a"]
        );
    }

    #[test]
    fn accepts_shared_dependencies_and_duplicate_edges() {
        assert!(validate_dependencies(&[
            ticket("tt-a", &["tt-b", "tt-c", "tt-b"]),
            ticket("tt-b", &["tt-d"]),
            ticket("tt-c", &["tt-d"]),
            ticket("tt-d", &[]),
        ])
        .is_empty());
    }

    #[test]
    fn validates_deep_graph_without_call_stack_recursion() {
        let tickets: Vec<_> = (0..20_000)
            .map(|index| {
                let id = format!("tt-{index:05}");
                let dependencies = if index == 19_999 {
                    vec![]
                } else {
                    vec![format!("tt-{:05}", index + 1)]
                };
                TicketMetadata {
                    depends_on: Some(dependencies),
                    ..ticket(&id, &[])
                }
            })
            .collect();
        assert!(validate_dependencies(&tickets).is_empty());
    }
}
