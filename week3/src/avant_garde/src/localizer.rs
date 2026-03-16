/// localizer.rs
/// Statistical object localization based on range readings from sensor nodes.
///
/// Two modes:
///   localize()          — single-frame deterministic estimate.
///   localize_windowed() — probabilistic trail over the full sensor window,
///                         returning one WeightedEstimate per time-step.

use crate::agent::Coordinate;

/// A reading from one active sensor node at a single time-step.
pub struct NodeReading {
    /// The known physical position of this node (same unit as distance below).
    pub pos: Coordinate,
    /// Measured distance from this node to the object.
    pub distance: f32,
}

/// A node's full sensor window, used by localize_windowed().
pub struct WindowNode {
    pub pos: Coordinate,
    /// Distance readings (index 0 = oldest).
    pub distances: Vec<f32>,
    /// Movement flags aligned with distances (0 = no motion, 1 = motion).
    pub movements: Vec<u8>,
}

/// One position estimate from the windowed localization.
/// Carries the result geometry plus a [0,1] weight (recency × confidence).
pub struct WeightedEstimate {
    pub result: LocalizationResult,
    /// 0.0 = old/uncertain … 1.0 = fresh/confident.
    pub weight: f32,
}


/// The result returned by the localizer, scaled to the caller's coordinate space.
pub enum LocalizationResult {
    /// No active nodes — nothing to display.
    NoData,

    /// Only one active node: the object is somewhere on a circle.
    Circle {
        center: Coordinate,
        radius: f32,
    },

    /// Two active nodes: two-circle intersection gives two candidate points.
    /// If the circles don't intersect (distance too large/small), we fall back
    /// to showing the midpoint with a small uncertainty radius.
    Arc {
        p1: Coordinate,
        p2: Coordinate,
        /// Midpoint of the chord between the two intersection points.
        midpoint: Coordinate,
    },

    /// Three or more active nodes: a single best-fit point estimate.
    Point {
        pos: Coordinate,
        /// Residual error (lower = more confident), in the same units as distance.
        residual: f32,
    },
}

/// Compute the localization result from a slice of active node readings
/// at a single point in time.
pub fn localize(readings: &[NodeReading]) -> LocalizationResult {
    match readings.len() {
        0 => LocalizationResult::NoData,
        1 => localize_one(&readings[0]),
        2 => localize_two(&readings[0], &readings[1]),
        _ => localize_multi(readings),
    }
}

/// Run localization over the last `window` samples from each node,
/// returning one `WeightedEstimate` per time-step (oldest first).
///
/// Only time-steps where ≥1 node detected motion are emitted.
/// Each estimate carries a weight = recency × (1 / (1 + residual)).
pub fn localize_windowed(nodes: &[WindowNode], window: usize) -> Vec<WeightedEstimate> {
    if nodes.is_empty() { return vec![]; }

    // Align all nodes to the shortest window.
    let min_len = nodes.iter().map(|n| n.distances.len()).min().unwrap_or(0);
    let w = window.min(min_len);
    if w == 0 { return vec![]; }

    let offset = min_len - w; // index into arrays for the oldest sample we use
    let mut out = Vec::with_capacity(w);

    for step in 0..w {
        let idx = offset + step;
        let readings: Vec<NodeReading> = nodes.iter()
            .filter(|n| n.movements[idx] > 0 && n.distances[idx] > 0.5)
            .map(|n| NodeReading { pos: n.pos, distance: n.distances[idx] })
            .collect();

        if readings.is_empty() { continue; }

        let result = localize(&readings);
        // Recency: oldest sample in window → 0, newest → 1
        let recency = (step + 1) as f32 / w as f32;
        // Confidence from residual (Point) or raw recency for Circle/Arc
        let confidence = match &result {
            LocalizationResult::Point { residual, .. } => {
                let r = *residual;
                (1.0 / (1.0 + r)).max(0.01)
            }
            LocalizationResult::Arc { .. } | LocalizationResult::Circle { .. } => 0.6,
            LocalizationResult::NoData => 0.0,
        };
        let weight = (recency * confidence).clamp(0.0, 1.0);
        out.push(WeightedEstimate { result, weight });
    }
    out
}

// ─── 1-node: circle ───────────────────────────────────────────────────────────

fn localize_one(r: &NodeReading) -> LocalizationResult {
    LocalizationResult::Circle {
        center: r.pos,
        radius: r.distance.max(0.1),
    }
}

// ─── 2-node: two-circle intersection ─────────────────────────────────────────

fn localize_two(a: &NodeReading, b: &NodeReading) -> LocalizationResult {
    let (x1, y1, r1) = (a.pos.x, a.pos.y, a.distance.max(0.01));
    let (x2, y2, r2) = (b.pos.x, b.pos.y, b.distance.max(0.01));

    let dx = x2 - x1;
    let dy = y2 - y1;
    let d = (dx * dx + dy * dy).sqrt();

    // Midpoint (raw, unweighted) as fallback
    let midpoint = Coordinate {
        x: (x1 + x2) / 2.0,
        y: (y1 + y2) / 2.0,
    };

    // Check if intersection exists: |r1 - r2| ≤ d ≤ r1 + r2
    if d < (r1 - r2).abs() || d > r1 + r2 || d < 1e-6 {
        // Circles don't intersect — return midpoint as the closest estimate
        let weighted_mid = weighted_midpoint(a, b);
        return LocalizationResult::Arc {
            p1: weighted_mid,
            p2: weighted_mid,
            midpoint: weighted_mid,
        };
    }

    // Distance from center-1 along the line joining the two centers,
    // to the radical axis (line through the intersection points).
    let a_len = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h_sq = r1 * r1 - a_len * a_len;

    if h_sq < 0.0 {
        // Numerical protection
        return LocalizationResult::Arc {
            p1: midpoint,
            p2: midpoint,
            midpoint,
        };
    }

    let h = h_sq.sqrt();

    // Point on the line between the two centers at distance a from center-1
    let mx = x1 + a_len * dx / d;
    let my = y1 + a_len * dy / d;

    // The two intersection points are perpendicular to the chord
    let p1 = Coordinate {
        x: mx + h * dy / d,
        y: my - h * dx / d,
    };
    let p2 = Coordinate {
        x: mx - h * dy / d,
        y: my + h * dx / d,
    };
    let chord_mid = Coordinate { x: mx, y: my };

    LocalizationResult::Arc {
        p1,
        p2,
        midpoint: chord_mid,
    }
}

/// Inverse-distance-weighted midpoint as a fallback for non-intersecting circles.
fn weighted_midpoint(a: &NodeReading, b: &NodeReading) -> Coordinate {
    let wa = 1.0 / (a.distance * a.distance + 1e-4);
    let wb = 1.0 / (b.distance * b.distance + 1e-4);
    let sum = wa + wb;
    Coordinate {
        x: (wa * a.pos.x + wb * b.pos.x) / sum,
        y: (wa * a.pos.y + wb * b.pos.y) / sum,
    }
}

// ─── 3+ nodes: weighted least-squares trilateration ──────────────────────────

/// Uses iterative gradient descent to minimise:
///   L(x,y) = Σ wᵢ · ((x - xᵢ)² + (y - yᵢ)² - rᵢ²)²
///
/// Weights wᵢ = 1 / (rᵢ² + ε) give closer nodes more influence.
fn localize_multi(readings: &[NodeReading]) -> LocalizationResult {
    // Initialise at the inverse-distance-weighted centroid of the node positions.
    let (mut x, mut y) = weighted_centroid(readings);

    let lr = 0.02f32;  // slightly larger lr — window calls 32× per frame
    let iters = 120;   // fewer iterations (still converges for well-placed nodes)

    for _ in 0..iters {
        let mut gx = 0.0f32;
        let mut gy = 0.0f32;

        for r in readings {
            let dx = x - r.pos.x;
            let dy = y - r.pos.y;
            let dist = (dx * dx + dy * dy).sqrt().max(1e-6);
            let ri = r.distance.max(0.01);

            // Gradient of (dist - ri)²
            let diff = dist - ri;
            let w = 1.0 / (ri * ri + 1e-4);
            gx += w * 2.0 * diff * (dx / dist);
            gy += w * 2.0 * diff * (dy / dist);
        }

        x -= lr * gx;
        y -= lr * gy;
    }

    // Compute residual (average distance error across all nodes)
    let residual = readings.iter().map(|r| {
        let dx = x - r.pos.x;
        let dy = y - r.pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        (dist - r.distance).abs()
    }).sum::<f32>() / readings.len() as f32;

    LocalizationResult::Point {
        pos: Coordinate { x, y },
        residual,
    }
}

/// Compute the inverse-distance-weighted centroid of node positions.
fn weighted_centroid(readings: &[NodeReading]) -> (f32, f32) {
    let mut sx = 0.0f32;
    let mut sy = 0.0f32;
    let mut sw = 0.0f32;
    for r in readings {
        let w = 1.0 / (r.distance * r.distance + 1e-4);
        sx += w * r.pos.x;
        sy += w * r.pos.y;
        sw += w;
    }
    if sw < 1e-9 { return (0.0, 0.0) }
    (sx / sw, sy / sw)
}
