use crate::builder::{BlockProcessor, IOBytes};
use crate::orchard::{OrchardDomain, OrchardHash, OrchardHasher};
use crate::{CTree, Witness, ORCHARD_ROOTS};
use ff::{Field, PrimeField};
use group::GroupEncoding;
use group::{Curve, Group};
use halo2_gadgets::sinsemilla::primitives::SINSEMILLA_S;
use incrementalmerkletree::bridgetree::BridgeTree;
use incrementalmerkletree::Tree;
use orchard::tree::MerkleHashOrchard;
use orchard::Anchor;
use pasta_curves::arithmetic::{CurveAffine, CurveExt};
use pasta_curves::pallas;
use pasta_curves::pallas::{Affine, Point};
use rand::rngs::OsRng;
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaChaRng;
use std::time::Instant;
use zcash_primitives::merkle_tree::Hashable;

fn make_test_data() -> Vec<([u8; 32], [u8; 32])> {
    let mut rng = ChaChaRng::seed_from_u64(0);
    let mut test_data = vec![];
    for i in 0..100_000 {
        if i % 1000 == 0 {
            println!("{}", i);
        }
        let left = pallas::Base::random(&mut rng).to_repr();
        let right = pallas::Base::random(&mut rng).to_repr();
        test_data.push((left, right));
    }
    test_data
}

pub fn test_pedersen_hash() {
    env_logger::init();
    let test_data = make_test_data();
    let oh = OrchardHasher::new();

    let librz = Instant::now();
    for (left, right) in test_data.iter() {
        let left1 = MerkleHashOrchard::from_bytes(&left).unwrap();
        let right1 = MerkleHashOrchard::from_bytes(&right).unwrap();
        let _res = MerkleHashOrchard::combine(4, &left1, &right1);
    }
    println!("librzc {}", librz.elapsed().as_millis());

    let wp = Instant::now();
    for (left, right) in test_data.iter() {
        let _hash = oh.hash_combine(4, &left, &right);
    }
    println!("wp {}", wp.elapsed().as_millis());
}

pub fn test_empty_tree() {
    env_logger::init();
    let mut rng = ChaChaRng::seed_from_u64(0);
    let mut test_data = vec![];
    for _ in 0..10 {
        let data = pallas::Base::random(&mut rng);
        test_data.push(data);
    }

    let mut tree = BridgeTree::<MerkleHashOrchard, 32>::new(8);
    for (i, d) in test_data.iter().enumerate() {
        tree.append(&MerkleHashOrchard::from_bytes(&d.to_repr()).unwrap());
        if i == 5 {
            tree.witness();
        }
    }
    tree.checkpoint();

    let root = tree.root(0).unwrap();
    let anchor: Anchor = root.into();
    println!("{}", hex::encode(anchor.to_bytes()));

    let mut bp = BlockProcessor::<OrchardDomain>::new(&CTree::new(), &[]);
    let mut nodes: Vec<_> = test_data.iter().map(|d| OrchardHash(d.to_repr())).collect();
    bp.add_nodes(&mut nodes, &[Witness::new(5, 0, None)]);
    let (tree2, witnesses) = bp.finalize();
    // let commitment_tree = tree2.to_commitment_tree();
    // let anchor = commitment_tree.root();
    let anchor2 = tree2.root(32, &ORCHARD_ROOTS);

    println!("{}", hex::encode(anchor2.0));

    assert_eq!(anchor.to_bytes(), anchor2.0);

    let path = tree.authentication_path(5.into(), &root).unwrap();
    for (i, p) in path.iter().enumerate() {
        println!("{}. {}", i, hex::encode(p.to_bytes()));
    }

    let path2 = witnesses[0].auth_path(32, &ORCHARD_ROOTS);
    for (i, p) in path2.iter().enumerate() {
        println!("{}. {}", i, hex::encode(p.0));
    }
}
