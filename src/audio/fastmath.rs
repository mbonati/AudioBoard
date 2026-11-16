//! Cheap stand-ins for the transcendental functions used per sample: they
//! live in the node SDK (`audioboard_node::fastmath`, with their accuracy
//! tests) so plugin nodes share them.

pub use audioboard_node::fastmath::*;
