//! Automatic layout. Blocks (nodes, or whole modules) go in columns by
//! signal flow, left to right; within a column each block moves level with
//! the ports it is wired to, so cables run as straight as possible.

use std::collections::{HashMap, HashSet};

use egui::{Pos2, Rect, Vec2, pos2, vec2};

use super::groups;
use super::widgets::cable_shape;
use super::nodes::HEADER_H;
use crate::audio::engine::NodeId;
use crate::graph::{Graph, Group, PortRef};

pub const GAP_X: f32 = 80.0;
pub const GAP_Y: f32 = 32.0;
/// Room kept for a cable crossing a column.
const WAYPOINT_H: f32 = 12.0;

/// A cable between two blocks. The offsets are the ports' heights within
/// their blocks, so blocks can be placed with the two ports level.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub from: usize,
    pub to: usize,
    pub from_dy: f32,
    pub to_dy: f32,
}

fn mean(values: impl Iterator<Item = f32>) -> Option<f32> {
    let (sum, n) = values.fold((0.0, 0), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / n as f32)
}

/// New top-left corner for every block, the arrangement's corner at the
/// origin. `current` positions break ties, so a tidy patch stays similar.
pub fn arrange(sizes: &[Vec2], current: &[Pos2], links: &[Link]) -> Vec<Pos2> {
    let n = sizes.len();
    if n == 0 {
        return Vec::new();
    }
    let links: Vec<Link> = links
        .iter()
        .copied()
        .filter(|l| l.from != l.to && l.from < n && l.to < n)
        .collect();
    let preds = |v: usize| links.iter().filter(move |l| l.to == v);
    let succs = |v: usize| links.iter().filter(move |l| l.from == v);

    // Topological order (the patch is acyclic; anything left over goes last).
    let mut indegree = vec![0usize; n];
    for l in &links {
        indegree[l.to] += 1;
    }
    let mut ready: Vec<usize> = (0..n).rev().filter(|&v| indegree[v] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(v) = ready.pop() {
        order.push(v);
        for l in succs(v) {
            indegree[l.to] -= 1;
            if indegree[l.to] == 0 {
                ready.push(l.to);
            }
        }
    }
    let placed: HashSet<usize> = order.iter().copied().collect();
    order.extend((0..n).filter(|v| !placed.contains(v)));

    // Columns: the longest path from a source…
    let mut column = vec![0usize; n];
    for &v in &order {
        for l in succs(v) {
            column[l.to] = column[l.to].max(column[v] + 1);
        }
    }
    // …except modulators (no inputs), which sit right before what they feed.
    for &v in order.iter().rev() {
        if preds(v).next().is_none()
            && let Some(c) = succs(v).map(|l| column[l.to]).min()
        {
            column[v] = c.saturating_sub(1);
        }
    }
    // Cables that skip columns get a waypoint in each column they cross, so
    // the blocks there leave room for them.
    let mut sizes = sizes.to_vec();
    let mut y: Vec<f32> = current.iter().map(|p| p.y).collect();
    let mut wired = Vec::with_capacity(links.len());
    // Column-skipping cables and their waypoints.
    let mut long: Vec<(Link, Vec<usize>)> = Vec::new();
    for l in &links {
        let (start, end) = (column[l.from], column[l.to]);
        let mut prev = (l.from, l.from_dy);
        let mut waypoints = Vec::new();
        for c in start + 1..end {
            let w = sizes.len();
            let t = (c - start) as f32 / (end - start) as f32;
            sizes.push(vec2(0.0, WAYPOINT_H));
            column.push(c);
            y.push(y[l.from] + l.from_dy + (y[l.to] + l.to_dy - y[l.from] - l.from_dy) * t - WAYPOINT_H * 0.5);
            wired.push(Link { from: prev.0, to: w, from_dy: prev.1, to_dy: WAYPOINT_H * 0.5 });
            prev = (w, WAYPOINT_H * 0.5);
            waypoints.push(w);
        }
        wired.push(Link { from: prev.0, to: l.to, from_dy: prev.1, to_dy: l.to_dy });
        if !waypoints.is_empty() {
            long.push((*l, waypoints));
        }
    }
    let links = wired;
    let preds = |v: usize| links.iter().filter(move |l| l.to == v);
    let succs = |v: usize| links.iter().filter(move |l| l.from == v);

    let columns = column.iter().max().map_or(1, |c| c + 1);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); columns];
    for (v, &c) in column.iter().enumerate() {
        layers[c].push(v);
    }

    let mut xs = Vec::with_capacity(columns);
    let mut x = 0.0;
    for layer in &layers {
        xs.push(x);
        x += layer.iter().map(|&v| sizes[v].x).fold(0.0, f32::max) + GAP_X;
    }

    // Rows: sweep the columns back and forth, pulling every block level
    // with its neighbours' ports; the last sweep favours the inputs.
    for pass in 0..5 {
        let forward = pass % 2 == 0;
        let cols: Vec<usize> = if forward { (0..columns).collect() } else { (0..columns).rev().collect() };
        for c in cols {
            let wanted: Vec<(usize, f32, f32)> = layers[c]
                .iter()
                .map(|&v| {
                    let from_inputs = mean(preds(v).map(|l| y[l.from] + l.from_dy - l.to_dy));
                    let from_outputs = mean(succs(v).map(|l| y[l.to] + l.to_dy - l.from_dy));
                    let w = if forward { from_inputs.or(from_outputs) } else { from_outputs.or(from_inputs) };
                    (v, w.unwrap_or(y[v]), 1.0)
                })
                .collect();
            place(&mut y, wanted, &sizes);
        }
    }
    // A cable is a curve between its ends, not a path through its waypoints:
    // widen each waypoint to the band the curve really sweeps in that column
    // and move the column's blocks out of it.
    let widths: Vec<f32> = layers
        .iter()
        .map(|layer| layer.iter().map(|&v| sizes[v].x).fold(0.0, f32::max))
        .collect();
    for _ in 0..3 {
        let mut held: HashMap<usize, f32> = HashMap::new();
        for (l, waypoints) in &long {
            let a = pos2(xs[column[l.from]] + sizes[l.from].x, y[l.from] + l.from_dy);
            let b = pos2(xs[column[l.to]], y[l.to] + l.to_dy);
            let curve = cable_shape(a, b, 1.0);
            let samples: Vec<Pos2> = (0..=48).map(|i| curve.sample(i as f32 / 48.0)).collect();
            for &w in waypoints {
                let (x0, x1) = (xs[column[w]], xs[column[w]] + widths[column[w]]);
                let ys = samples.iter().filter(|p| p.x >= x0 && p.x <= x1).map(|p| p.y);
                let (lo, hi) = ys.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), y| (lo.min(y), hi.max(y)));
                if lo <= hi {
                    sizes[w].y = hi - lo + WAYPOINT_H;
                    held.insert(w, lo - WAYPOINT_H * 0.5);
                }
            }
        }
        for layer in layers.iter().filter(|layer| layer.iter().any(|v| held.contains_key(v))) {
            let wanted = layer
                .iter()
                .map(|&v| match held.get(&v) {
                    Some(&row) => (v, row, 1000.0),
                    None => (v, y[v], 1.0),
                })
                .collect();
            place(&mut y, wanted, &sizes);
        }
    }

    // Finally line sources up with what they feed, holding everything that
    // has inputs where it is.
    for c in (0..columns).rev() {
        let wanted: Vec<(usize, f32, f32)> = layers[c]
            .iter()
            .map(|&v| match (preds(v).next(), mean(succs(v).map(|l| y[l.to] + l.to_dy - l.from_dy))) {
                (None, Some(w)) => (v, w, 1.0),
                _ => (v, y[v], 1000.0),
            })
            .collect();
        place(&mut y, wanted, &sizes);
    }

    let top = y[..n].iter().copied().fold(f32::INFINITY, f32::min);
    (0..n).map(|v| pos2(xs[column[v]], y[v] - top)).collect()
}

/// Stacks a column's blocks in the order of their wanted rows, as close to
/// those rows as possible without overlapping: weighted isotonic regression
/// (pool adjacent violators) on the rows minus the space above each block.
/// `wanted`: (block, row, weight).
fn place(y: &mut [f32], mut wanted: Vec<(usize, f32, f32)>, sizes: &[Vec2]) {
    wanted.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut above = 0.0;
    // Pools of (weighted sum, total weight, count).
    let mut pools: Vec<(f32, f32, usize)> = Vec::new();
    for &(v, row, weight) in &wanted {
        pools.push(((row - above) * weight, weight, 1));
        above += sizes[v].y + GAP_Y;
        while let [.., (s1, w1, c1), (s2, w2, c2)] = pools[..] {
            if s1 / w1 <= s2 / w2 {
                break;
            }
            pools.truncate(pools.len() - 2);
            pools.push((s1 + s2, w1 + w2, c1 + c2));
        }
    }
    let mut above = 0.0;
    let mut i = 0;
    for (sum, weight, count) in pools {
        for _ in 0..count {
            let v = wanted[i].0;
            y[v] = sum / weight + above;
            above += sizes[v].y + GAP_Y;
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// The patch as blocks
// ---------------------------------------------------------------------------

/// Where every node should go. `only`: lay out just these nodes (modules
/// they belong to move whole); otherwise everything. Modules are also
/// tidied inside. The arrangement keeps its old top-left corner.
pub fn plan(graph: &Graph, only: Option<&HashSet<NodeId>>) -> Vec<(NodeId, Pos2)> {
    let in_scope = |id: NodeId| only.is_none_or(|s| s.contains(&id));
    let mut pos: HashMap<NodeId, Pos2> = graph.nodes.iter().map(|n| (n.id, n.pos)).collect();
    let modules: Vec<&Group> = graph
        .groups
        .iter()
        .filter(|g| g.members.iter().any(|m| in_scope(*m)))
        .collect();

    // Inside each module first, anchored at its top-left node corner.
    for g in &modules {
        let blocks: Vec<Block> = g.members.iter().map(|&m| Block::Node(m)).collect();
        arrange_blocks(graph, &blocks, &mut pos);
    }

    // Then free nodes and modules as blocks.
    let grouped: HashSet<NodeId> = graph.groups.iter().flat_map(|g| g.members.iter().copied()).collect();
    let mut blocks: Vec<Block> = modules.iter().map(|g| Block::Module(g)).collect();
    blocks.extend(
        graph
            .nodes
            .iter()
            .filter(|n| in_scope(n.id) && !grouped.contains(&n.id))
            .map(|n| Block::Node(n.id)),
    );
    arrange_blocks(graph, &blocks, &mut pos);

    graph
        .nodes
        .iter()
        .filter(|n| in_scope(n.id) || modules.iter().any(|g| g.members.contains(&n.id)))
        .map(|n| (n.id, pos[&n.id]))
        .collect()
}

/// A block's rect and its visible ports: (port, is output, world position).
type Geometry = (Rect, Vec<(PortRef, bool, Pos2)>);

enum Block<'a> {
    Node(NodeId),
    Module(&'a Group),
}

impl Block<'_> {
    fn members(&self) -> Vec<NodeId> {
        match self {
            Block::Node(id) => vec![*id],
            Block::Module(g) => g.members.clone(),
        }
    }
}

/// Rect of a node at its planned position.
fn node_rect(graph: &Graph, pos: &HashMap<NodeId, Pos2>, id: NodeId) -> Option<Rect> {
    Some(Rect::from_min_size(pos[&id], graph.node(id)?.size()))
}

/// A block's rect, and where each of its visible ports is (world space).
fn block_geometry(
    graph: &Graph,
    pos: &HashMap<NodeId, Pos2>,
    block: &Block,
) -> Option<Geometry> {
    let port_at = |pr: PortRef, output: bool| {
        let n = graph.node(pr.node)?;
        let local = if output { n.output_pos(pr.port) } else { n.input_pos(pr.port) } - n.pos;
        Some(pos[&pr.node] + local)
    };
    let node_ports = |id: NodeId| -> Vec<(PortRef, bool, Pos2)> {
        let Some(n) = graph.node(id) else { return Vec::new() };
        let ins = n.input_ports().into_iter().map(|port| (PortRef { node: id, port }, false));
        let outs = n.output_ports().into_iter().map(|port| (PortRef { node: id, port }, true));
        ins.chain(outs)
            .filter_map(|(pr, out)| Some((pr, out, port_at(pr, out)?)))
            .collect()
    };
    match block {
        Block::Node(id) => Some((node_rect(graph, pos, *id)?, node_ports(*id))),
        Block::Module(g) => {
            let bounds = g
                .members
                .iter()
                .filter_map(|m| node_rect(graph, pos, *m))
                .reduce(|a, b| a.union(b))?;
            let frame = Rect::from_min_max(
                bounds.min - vec2(groups::PAD, groups::PAD + HEADER_H),
                bounds.max + vec2(groups::PAD, groups::PAD),
            );
            if !g.collapsed {
                let ports = g.members.iter().flat_map(|m| node_ports(*m)).collect();
                return Some((frame, ports));
            }
            let (ins, outs) = graph.exposed_ports(g);
            let card = Rect::from_min_size(frame.min, groups::card_size(ins.len(), outs.len()));
            let ports = ins
                .iter()
                .enumerate()
                .map(|(k, pr)| (*pr, false, card.min + groups::card_port(k, false)))
                .chain(outs.iter().enumerate().map(|(k, pr)| (*pr, true, card.min + groups::card_port(k, true))))
                .collect();
            Some((card, ports))
        }
    }
}

/// Lays out `blocks`, moving their nodes in `pos`.
fn arrange_blocks(graph: &Graph, blocks: &[Block], pos: &mut HashMap<NodeId, Pos2>) {
    let geometry: Vec<Geometry> =
        blocks.iter().map(|b| block_geometry(graph, pos, b).unwrap_or((Rect::ZERO, Vec::new()))).collect();
    let mut owner: HashMap<(PortRef, bool), (usize, f32)> = HashMap::new();
    for (i, (rect, ports)) in geometry.iter().enumerate() {
        for &(pr, output, at) in ports {
            owner.insert((pr, output), (i, at.y - rect.top()));
        }
    }
    let links: Vec<Link> = graph
        .cables
        .iter()
        .filter_map(|c| {
            let &(from, from_dy) = owner.get(&(c.from, true))?;
            let &(to, to_dy) = owner.get(&(c.to, false))?;
            Some(Link { from, to, from_dy, to_dy })
        })
        .collect();

    let sizes: Vec<Vec2> = geometry.iter().map(|(r, _)| r.size()).collect();
    let current: Vec<Pos2> = geometry.iter().map(|(r, _)| r.min).collect();
    let corner = geometry
        .iter()
        .map(|(r, _)| r.min)
        .reduce(|a, b| a.min(b))
        .unwrap_or_default()
        .to_vec2();
    let placed = arrange(&sizes, &current, &links);
    for (i, block) in blocks.iter().enumerate() {
        let delta = placed[i] + corner - current[i];
        for m in block.members() {
            if let Some(p) = pos.get_mut(&m) {
                *p += delta;
            }
        }
    }
}
