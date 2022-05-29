use crate::builder::{Domain, SaplingDomain};
use crate::{CTree, Witness};
use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
use zcash_primitives::sapling::Node;

#[allow(dead_code)]
pub fn print_node(n: &Node) {
    println!("{:?}", hex::encode(n.repr));
}

#[allow(dead_code)]
pub fn print_tree(t: &CommitmentTree<Node>) {
    println!("{:?}", t.left.map(|n| hex::encode(n.repr)));
    println!("{:?}", t.right.map(|n| hex::encode(n.repr)));
    for p in t.parents.iter() {
        println!("{:?}", p.map(|n| hex::encode(n.repr)));
    }
}

#[allow(dead_code)]
pub fn print_witness(w: &IncrementalWitness<Node>) {
    println!("Tree");
    print_tree(&w.tree);
    println!("Filled");
    for n in w.filled.iter() {
        print_node(n);
    }
    println!("Cursor");
    w.cursor.as_ref().map(|c| print_tree(c));
}

pub fn print_ctree(t: &CTree<SaplingDomain>) {
    println!("Tree");
    println!("{:?}", t.left.map(|n| hex::encode(n.0.repr)));
    println!("{:?}", t.right.map(|n| hex::encode(n.0.repr)));
    for p in t.parents.iter() {
        println!("{:?}", p.map(|n| hex::encode(n.0.repr)));
    }
}

#[allow(dead_code)]
pub fn print_witness2(w: &Witness<SaplingDomain>) {
    let t = &w.tree;
    print_ctree(t);
    println!("Filled");
    for n in w.filled.iter() {
        print_node(&n.0);
    }
    let t = &w.cursor;
    println!("Cursor");
    println!("{:?}", t.left.map(|n| hex::encode(n.0.repr)));
    println!("{:?}", t.right.map(|n| hex::encode(n.0.repr)));
    for p in t.parents.iter() {
        println!("{:?}", p.map(|n| hex::encode(n.0.repr)));
    }
}
