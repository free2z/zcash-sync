use crate::commitment::{CTree, Witness};
use crate::hash::{pedersen_hash, pedersen_hash_inner};
use ff::PrimeField;
use group::Curve;
use jubjub::{AffinePoint, ExtendedPoint};
use rayon::prelude::IntoParallelIterator;
use rayon::prelude::*;
use std::io::{Read, Write};
use std::marker::PhantomData;
use zcash_primitives::merkle_tree::HashSer;
use zcash_primitives::sapling::Note;

pub trait IOBytes: Sized {
    fn new(hash: [u8; 32]) -> Self;
    fn write<W: Write>(&self, w: W) -> std::io::Result<()>;
    fn read<R: Read>(r: R) -> std::io::Result<Self>;
    fn uncommitted() -> Self;
}

pub trait Domain: Clone {
    type Node: Clone + Send + Sync + Copy + IOBytes;
    fn node_combine(depth: usize, left: &Self::Node, right: &Self::Node) -> Self::Node;
}

#[derive(Copy, Clone, PartialEq)]
pub struct SaplingNode(pub zcash_primitives::sapling::Node);
impl IOBytes for SaplingNode {
    fn new(hash: [u8; 32]) -> Self {
        SaplingNode(zcash_primitives::sapling::Node::new(hash))
    }

    fn write<W: Write>(&self, w: W) -> std::io::Result<()> {
        self.0.write(w)
    }

    fn read<R: Read>(r: R) -> std::io::Result<Self> {
        zcash_primitives::sapling::Node::read(r).map(SaplingNode)
    }

    fn uncommitted() -> Self {
        let n = Note::uncommitted().to_repr();
        Self::new(n)
    }
}

#[derive(Clone)]
pub struct SaplingDomain;
impl Domain for SaplingDomain {
    type Node = SaplingNode;

    fn node_combine(depth: usize, left: &Self::Node, right: &Self::Node) -> Self::Node {
        Self::Node::new(pedersen_hash(depth as u8, &left.0.repr, &right.0.repr))
    }
}

// #[inline(always)]
// fn batch_node_combine1(depth: usize, left: &Node, right: &Node) -> ExtendedPoint {
//     // Node::new(pedersen_hash(depth as u8, &left.repr, &right.repr))
//     ExtendedPoint::from(pedersen_hash_inner(depth as u8, &left.repr, &right.repr))
// }
//

// #[inline(always)]
// fn node_combine(depth: usize, left: &Node, right: &Node) -> Node {
// }

trait Builder<D: Domain> {
    type Context;
    type Output;

    fn collect(&mut self, commitments: &[D::Node], context: &Self::Context) -> usize;
    fn up(&mut self);
    fn finished(&self) -> bool;
    fn finalize(self, context: &Self::Context) -> Self::Output;
}

struct CTreeBuilder<D: Domain> {
    left: Option<D::Node>,
    right: Option<D::Node>,
    prev_tree: CTree<D>,
    next_tree: CTree<D>,
    start: usize,
    total_len: usize,
    depth: usize,
    offset: Option<D::Node>,
    first_block: bool,
}

impl<D: Domain> Builder<D> for CTreeBuilder<D> {
    type Context = ();
    type Output = CTree<D>;

    fn collect(&mut self, commitments: &[D::Node], _context: &()) -> usize {
        assert!(self.right.is_none() || self.left.is_some()); // R can't be set without L

        let offset: Option<D::Node>;
        let m: usize;

        if self.left.is_some() && self.right.is_none() {
            offset = self.left;
            m = commitments.len() + 1;
        } else {
            offset = None;
            m = commitments.len();
        };

        let n = if self.total_len > 0 {
            if self.depth == 0 {
                if m % 2 == 0 {
                    self.next_tree.left = Some(*Self::get(commitments, m - 2, &offset));
                    self.next_tree.right = Some(*Self::get(commitments, m - 1, &offset));
                    m - 2
                } else {
                    self.next_tree.left = Some(*Self::get(commitments, m - 1, &offset));
                    self.next_tree.right = None;
                    m - 1
                }
            } else {
                if m % 2 == 0 {
                    self.next_tree.parents.push(None);
                    m
                } else {
                    let last_node = Self::get(commitments, m - 1, &offset);
                    self.next_tree.parents.push(Some(*last_node));
                    m - 1
                }
            }
        } else {
            0
        };
        assert_eq!(n % 2, 0);

        self.offset = offset;
        n
    }

    fn up(&mut self) {
        let h = if self.left.is_some() && self.right.is_some() {
            Some(D::node_combine(
                self.depth,
                &self.left.unwrap(),
                &self.right.unwrap(),
            ))
        } else {
            None
        };
        let (l, r) = match self.prev_tree.parents.get(self.depth) {
            Some(Some(p)) => (Some(*p), h),
            Some(None) => (h, None),
            None => (h, None),
        };

        self.left = l;
        self.right = r;

        assert!(self.start % 2 == 0 || self.offset.is_some());
        self.start /= 2;

        self.depth += 1;
    }

    fn finished(&self) -> bool {
        self.depth >= self.prev_tree.parents.len() && self.left.is_none() && self.right.is_none()
    }

    fn finalize(self, _context: &()) -> CTree<D> {
        if self.total_len > 0 {
            self.next_tree
        } else {
            self.prev_tree
        }
    }
}

impl<D: Domain> CTreeBuilder<D> {
    fn new(prev_tree: &CTree<D>, len: usize, first_block: bool) -> Self {
        let start = prev_tree.get_position();
        CTreeBuilder {
            left: prev_tree.left,
            right: prev_tree.right,
            prev_tree: prev_tree.clone(),
            next_tree: CTree::new(),
            start,
            total_len: len,
            depth: 0,
            offset: None,
            first_block,
        }
    }

    #[inline(always)]
    fn get_opt<'a>(
        commitments: &'a [D::Node],
        index: usize,
        offset: &'a Option<D::Node>,
    ) -> Option<&'a D::Node> {
        if offset.is_some() {
            if index > 0 {
                commitments.get(index - 1)
            } else {
                offset.as_ref()
            }
        } else {
            commitments.get(index)
        }
    }

    #[inline(always)]
    fn get<'a>(
        commitments: &'a [D::Node],
        index: usize,
        offset: &'a Option<D::Node>,
    ) -> &'a D::Node {
        Self::get_opt(commitments, index, offset).unwrap()
    }

    fn adjusted_start(&self, prev: &Option<D::Node>) -> usize {
        if prev.is_some() {
            self.start - 1
        } else {
            self.start
        }
    }
}

fn combine_level<D: Domain>(
    commitments: &mut [D::Node],
    offset: Option<D::Node>,
    n: usize,
    depth: usize,
) -> usize {
    assert_eq!(n % 2, 0);

    let nn = n / 2;
    let next_level: Vec<_> = (0..nn)
        .into_par_iter()
        .map(|i| {
            D::node_combine(
                depth,
                CTreeBuilder::<D>::get(commitments, 2 * i, &offset),
                CTreeBuilder::<D>::get(commitments, 2 * i + 1, &offset),
            )
        })
        .collect();

    commitments[0..nn].copy_from_slice(&next_level);
    nn
}

struct WitnessBuilder<D: Domain> {
    witness: Witness<D>,
    p: usize,
    inside: bool,
    _phantom: PhantomData<D>,
}

impl<D: Domain> WitnessBuilder<D> {
    fn new(tree_builder: &CTreeBuilder<D>, prev_witness: &Witness<D>, count: usize) -> Self {
        let position = prev_witness.position;
        let inside = position >= tree_builder.start && position < tree_builder.start + count;
        WitnessBuilder {
            witness: prev_witness.clone(),
            p: position,
            inside,
            _phantom: PhantomData::default(),
        }
    }
}

impl<D: Domain> Builder<D> for WitnessBuilder<D> {
    type Context = CTreeBuilder<D>;
    type Output = Witness<D>;

    fn collect(&mut self, commitments: &[D::Node], context: &CTreeBuilder<D>) -> usize {
        let offset = context.offset;
        let depth = context.depth;

        let tree = &mut self.witness.tree;

        if self.inside {
            let rp = self.p - context.adjusted_start(&offset);
            if depth == 0 {
                if self.p % 2 == 1 {
                    tree.left = Some(*CTreeBuilder::<D>::get(commitments, rp - 1, &offset));
                    tree.right = Some(*CTreeBuilder::<D>::get(commitments, rp, &offset));
                } else {
                    tree.left = Some(*CTreeBuilder::<D>::get(commitments, rp, &offset));
                    tree.right = None;
                }
            } else {
                if self.p % 2 == 1 {
                    tree.parents
                        .push(Some(*CTreeBuilder::<D>::get(commitments, rp - 1, &offset)));
                } else if self.p != 0 {
                    tree.parents.push(None);
                }
            }
        }

        let right = if depth != 0 && !context.first_block {
            context.right
        } else {
            None
        };
        // println!("D {}", depth);
        // println!("O {:?}", offset.map(|r| hex::encode(r.repr)));
        // println!("R {:?}", right.map(|r| hex::encode(r.repr)));
        // for c in commitments.iter() {
        //     println!("{}", hex::encode(c.repr));
        // }
        let p1 = self.p + 1;
        // println!("P {} P1 {} S {} AS {}", self.p, p1, context.start, context.adjusted_start(&right));
        let has_p1 = p1 >= context.adjusted_start(&right) && p1 < context.start + commitments.len();
        if has_p1 {
            let p1 =
                CTreeBuilder::<D>::get(commitments, p1 - context.adjusted_start(&right), &right);
            if depth == 0 {
                if tree.right.is_none() {
                    self.witness.filled.push(*p1);
                }
            } else {
                if depth - 1 >= tree.parents.len() || tree.parents[depth - 1].is_none() {
                    self.witness.filled.push(*p1);
                }
            }
        }
        0
    }

    fn up(&mut self) {
        self.p /= 2;
    }

    fn finished(&self) -> bool {
        false
    }

    fn finalize(mut self, context: &CTreeBuilder<D>) -> Witness<D> {
        if context.total_len == 0 {
            self.witness.cursor = CTree::new();

            let mut final_position = context.prev_tree.get_position() as u32;
            let mut witness_position = self.witness.tree.get_position() as u32;
            assert_ne!(witness_position, 0);
            witness_position = witness_position - 1;

            // look for first not equal bit in MSB order
            final_position = final_position.reverse_bits();
            witness_position = witness_position.reverse_bits();
            let mut bit: i32 = 31;
            // reverse bits because it is easier to do in LSB
            // it should not underflow because these numbers are not equal
            while bit >= 0 {
                if final_position & 1 != witness_position & 1 {
                    break;
                }
                final_position >>= 1;
                witness_position >>= 1;
                bit -= 1;
            }
            // look for the first bit set in final_position after
            final_position >>= 1;
            bit -= 1;
            while bit >= 0 {
                if final_position & 1 == 1 {
                    break;
                }
                final_position >>= 1;
                bit -= 1;
            }
            if bit >= 0 {
                self.witness.cursor = context.prev_tree.clone_trimmed(bit as usize)
            }
        }
        self.witness
    }
}

#[allow(dead_code)]
pub fn advance_tree<D: Domain>(
    prev_tree: &CTree<D>,
    prev_witnesses: &[Witness<D>],
    mut commitments: &mut [D::Node],
    first_block: bool,
) -> (CTree<D>, Vec<Witness<D>>) {
    let mut builder = CTreeBuilder::<D>::new(&prev_tree, commitments.len(), first_block);
    let mut witness_builders: Vec<_> = prev_witnesses
        .iter()
        .map(|witness| WitnessBuilder::new(&builder, &witness, commitments.len()))
        .collect();
    while !commitments.is_empty() || !builder.finished() {
        let n = builder.collect(commitments, &());
        for b in witness_builders.iter_mut() {
            b.collect(commitments, &builder);
        }
        let nn = combine_level::<D>(commitments, builder.offset, n, builder.depth);
        builder.up();
        for b in witness_builders.iter_mut() {
            b.up();
        }
        commitments = &mut commitments[0..nn];
    }

    let witnesses = witness_builders
        .into_iter()
        .map(|b| b.finalize(&builder))
        .collect();
    let tree = builder.finalize(&());
    (tree, witnesses)
}

pub struct BlockProcessor<D: Domain> {
    prev_tree: CTree<D>,
    prev_witnesses: Vec<Witness<D>>,
    first_block: bool,
}

impl<D: Domain> BlockProcessor<D> {
    pub fn new(prev_tree: &CTree<D>, prev_witnesses: &[Witness<D>]) -> BlockProcessor<D> {
        BlockProcessor {
            prev_tree: prev_tree.clone(),
            prev_witnesses: prev_witnesses.to_vec(),
            first_block: true,
        }
    }

    pub fn add_nodes(&mut self, nodes: &mut [D::Node], new_witnesses: &[Witness<D>]) {
        if nodes.is_empty() {
            return;
        }
        self.prev_witnesses.extend_from_slice(new_witnesses);
        let (t, ws) = advance_tree(
            &self.prev_tree,
            &self.prev_witnesses,
            nodes,
            self.first_block,
        );
        self.first_block = false;
        self.prev_tree = t;
        self.prev_witnesses = ws;
    }

    pub fn finalize(self) -> (CTree<D>, Vec<Witness<D>>) {
        if self.first_block {
            (self.prev_tree, self.prev_witnesses)
        } else {
            let (t, ws) = advance_tree(&self.prev_tree, &self.prev_witnesses, &mut [], false);
            (t, ws)
        }
    }
}

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    use crate::builder::{
        advance_tree, BlockProcessor, Domain, IOBytes, SaplingDomain, SaplingNode,
    };
    use crate::chain::DecryptedNote;
    use crate::commitment::{CTree, Witness};
    use crate::print::{print_ctree, print_tree, print_witness, print_witness2};
    use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
    use zcash_primitives::sapling::Node;

    fn make_nodes(p: usize, len: usize) -> Vec<<SaplingDomain as Domain>::Node> {
        let nodes: Vec<_> = (p..p + len)
            .map(|v| {
                let mut bb = [0u8; 32];
                bb[0..8].copy_from_slice(&v.to_be_bytes());
                SaplingNode::new(bb)
            })
            .collect();
        nodes
    }

    fn make_witnesses<D: Domain>(p: usize, len: usize) -> Vec<Witness<D>> {
        let witnesses: Vec<_> = (p..p + len).map(|v| Witness::new(v, 0, None)).collect();
        witnesses
    }

    fn update_witnesses1(
        tree: &mut CommitmentTree<Node>,
        ws: &mut Vec<IncrementalWitness<Node>>,
        nodes: &[SaplingNode],
    ) {
        for n in nodes.iter() {
            tree.append(n.clone().0).unwrap();
            for w in ws.iter_mut() {
                w.append(n.clone().0).unwrap();
            }
            let w = IncrementalWitness::<Node>::from_tree(&tree);
            ws.push(w);
        }
    }

    fn compare_witness(w1: &IncrementalWitness<Node>, w2: &Witness<SaplingDomain>) {
        let mut bb1: Vec<u8> = vec![];
        w1.write(&mut bb1).unwrap();
        let mut bb2: Vec<u8> = vec![];
        w2.write(&mut bb2).unwrap();

        if bb1 != bb2 {
            print_witness(&w1);
            print_witness2(&w2);

            assert!(false);
        }
    }

    #[test]
    fn test_simple() {
        let v = [0u8; 32];
        let mut bp = BlockProcessor::<SaplingDomain>::new(&CTree::new(), &[]);
        let mut nodes = [SaplingNode::new(v)];
        bp.add_nodes(&mut [], &[]);
        bp.add_nodes(&mut nodes, &[Witness::new(0, 0, None)]);
        bp.finalize();
    }

    #[test]
    fn test_bp_1run() {
        for n1 in 0..=40 {
            for n2 in 0..=40 {
                println!("{} {}", n1, n2);
                let mut bp = BlockProcessor::<SaplingDomain>::new(&CTree::new(), &[]);
                let mut tree1: CommitmentTree<Node> = CommitmentTree::empty();
                let mut ws1: Vec<IncrementalWitness<Node>> = vec![];

                let mut nodes = make_nodes(0, n1);
                update_witnesses1(&mut tree1, &mut ws1, &nodes);
                bp.add_nodes(&mut nodes, &make_witnesses(0, n1));

                let mut nodes = make_nodes(n1, n2);
                update_witnesses1(&mut tree1, &mut ws1, &nodes);
                bp.add_nodes(&mut nodes, &make_witnesses(n1, n2));

                let (_, ws2) = bp.finalize();

                for (i, (w1, w2)) in ws1.iter().zip(ws2.iter()).enumerate() {
                    println!("Compare {}", i);
                    compare_witness(w1, w2);
                }
            }
        }
    }

    #[test]
    fn test_bp_2run() {
        for n1 in 0..=40 {
            for n2 in 0..=40 {
                println!("{} {}", n1, n2);
                let mut tree1: CommitmentTree<Node> = CommitmentTree::empty();
                let mut ws1: Vec<IncrementalWitness<Node>> = vec![];
                let mut tree2 = CTree::new();
                let mut ws2: Vec<Witness<SaplingDomain>> = vec![];

                {
                    let mut bp = BlockProcessor::new(&tree2, &ws2);
                    let mut nodes = make_nodes(0, n1);
                    update_witnesses1(&mut tree1, &mut ws1, &nodes);
                    bp.add_nodes(&mut nodes, &make_witnesses(0, n1));
                    let (t2, w2) = bp.finalize();
                    tree2 = t2;
                    ws2 = w2;
                }

                {
                    let mut bp = BlockProcessor::new(&tree2, &ws2);
                    let mut nodes = make_nodes(n1, n2);
                    update_witnesses1(&mut tree1, &mut ws1, &nodes);
                    bp.add_nodes(&mut nodes, &make_witnesses(n1, n2));
                    let (_t2, w2) = bp.finalize();
                    ws2 = w2;
                }

                for (i, (w1, w2)) in ws1.iter().zip(ws2.iter()).enumerate() {
                    println!("Compare {}", i);
                    compare_witness(w1, w2);
                }
            }
        }
    }

    #[test]
    fn test_advance_tree_equal_blocks() {
        for num_nodes in 1..=10 {
            for num_chunks in 1..=10 {
                test_advance_tree_helper(num_nodes, num_chunks, 100.0, None);
            }
        }
    }

    #[test]
    fn test_advance_tree_unequal_blocks() {
        for num_nodes1 in 0..=30 {
            for num_nodes2 in 0..=30 {
                println!("TESTING {} {}", num_nodes1, num_nodes2);
                let (t, ws) = test_advance_tree_helper(num_nodes1, 1, 100.0, None);
                test_advance_tree_helper(num_nodes2, 1, 100.0, Some((t, ws)));
            }
        }
    }

    #[test]
    fn test_small_blocks() {
        for num_nodes1 in 1..=30 {
            println!("TESTING {}", num_nodes1);
            test_advance_tree_helper(num_nodes1, 1, 100.0, None);
        }
    }

    #[test]
    fn test_tree() {
        test_advance_tree_helper(4, 1, 100.0, None);

        // test_advance_tree_helper(2, 10, 100.0);
        // test_advance_tree_helper(1, 40, 100.0);
        // test_advance_tree_helper(10, 2, 100.0);
    }

    fn test_advance_tree_helper(
        num_nodes: usize,
        num_chunks: usize,
        witness_percent: f64,
        initial: Option<(CTree<SaplingDomain>, Vec<Witness<SaplingDomain>>)>,
    ) -> (CTree<SaplingDomain>, Vec<Witness<SaplingDomain>>) {
        let witness_freq = (100.0 / witness_percent) as usize;

        let mut tree1: CommitmentTree<Node> = CommitmentTree::empty();
        let mut tree2 = CTree::new();
        let mut ws: Vec<IncrementalWitness<Node>> = vec![];
        let mut ws2: Vec<Witness<SaplingDomain>> = vec![];
        if let Some((t0, ws0)) = initial {
            tree2 = t0;
            ws2 = ws0;

            let mut bb: Vec<u8> = vec![];
            tree2.write(&mut bb).unwrap();
            tree1 = CommitmentTree::<Node>::read(&*bb).unwrap();

            for w in ws2.iter() {
                bb = vec![];
                w.write(&mut bb).unwrap();
                let w1 = IncrementalWitness::<Node>::read(&*bb).unwrap();
                ws.push(w1);
            }
        }
        let p0 = tree2.get_position();
        let mut bp = BlockProcessor::new(&tree2, &ws2);

        for i in 0..num_chunks {
            println!("{}", i);
            let mut nodes: Vec<_> = vec![];
            let mut ws2: Vec<Witness<SaplingDomain>> = vec![];
            for j in 0..num_nodes {
                let mut bb = [0u8; 32];
                let v = i * num_nodes + j + p0;
                bb[0..8].copy_from_slice(&v.to_be_bytes());
                let node = Node::new(bb);
                let node2 = SaplingNode::new(bb);
                tree1.append(node).unwrap();
                for w in ws.iter_mut() {
                    w.append(node).unwrap();
                }
                if v % witness_freq == 0 {
                    // if v == 0 {
                    let w = IncrementalWitness::from_tree(&tree1);
                    ws.push(w);
                    ws2.push(Witness::new(v, 0, None));
                }
                nodes.push(node2);
            }

            bp.add_nodes(&mut nodes, &ws2);
        }

        let (new_tree, new_witnesses) = bp.finalize();
        tree2 = new_tree;
        ws2 = new_witnesses;

        // check final state
        let mut bb1: Vec<u8> = vec![];
        tree1.write(&mut bb1).unwrap();

        let mut bb2: Vec<u8> = vec![];
        tree2.write(&mut bb2).unwrap();

        let equal = bb1.as_slice() == bb2.as_slice();
        if !equal {
            println!("FAILED FINAL STATE");
            print_tree(&tree1);
            print_ctree(&tree2);
        }

        println!("# witnesses = {}", ws.len());

        // check witnesses
        let mut failed_index: Option<usize> = None;
        for (i, (w1, w2)) in ws.iter().zip(&ws2).enumerate() {
            let mut bb1: Vec<u8> = vec![];
            w1.write(&mut bb1).unwrap();

            let mut bb2: Vec<u8> = vec![];
            w2.write(&mut bb2).unwrap();

            if bb1.as_slice() != bb2.as_slice() {
                failed_index = Some(i);
                println!("FAILED AT {}", i);
                println!("GOOD");
                print_witness(&w1);
                if let Some(ref c) = w1.cursor {
                    print_tree(c);
                } else {
                    println!("NONE");
                }

                println!("BAD");
                print_witness2(&w2);
            }
            assert!(equal && failed_index.is_none());
        }

        (tree2, ws2)
    }
}
