#![allow(non_snake_case)]
use crate::builder::{Domain, IOBytes};
use ff::PrimeField;
use group::Curve;
use halo2_gadgets::sinsemilla::primitives::SINSEMILLA_S;
use lazy_static::lazy_static;
use pasta_curves::arithmetic::{CurveAffine, CurveExt};
use pasta_curves::pallas::{self, Affine, Point};
use std::io::{Read, Write};

lazy_static! {
    static ref ORCHARD_HASHER: OrchardHasher = OrchardHasher::new();
    pub static ref ORCHARD_ROOTS: Vec<OrchardHash> = {
        let h = OrchardHasher::new();
        h.empty_roots(32)
    };
}

#[derive(Copy, Clone)]
pub struct OrchardHash(pub [u8; 32]);

impl IOBytes for OrchardHash {
    fn new(hash: [u8; 32]) -> Self {
        OrchardHash(hash)
    }

    fn write<W: Write>(&self, mut w: W) -> std::io::Result<()> {
        w.write_all(&self.0)
    }

    fn read<R: Read>(mut r: R) -> std::io::Result<Self> {
        let mut buf = [0u8; 32];
        r.read(&mut buf)?;
        Ok(OrchardHash(buf))
    }

    fn uncommitted() -> Self {
        OrchardHash(pallas::Base::from(2).to_repr())
    }
}

#[derive(Clone)]
pub struct OrchardDomain;
impl Domain for OrchardDomain {
    type Node = OrchardHash;

    fn node_combine(depth: usize, left: &Self::Node, right: &Self::Node) -> Self::Node {
        OrchardHash(ORCHARD_HASHER.hash_combine(depth as u8, &left.0, &right.0))
    }
}

pub const Q_PERSONALIZATION: &str = "z.cash:SinsemillaQ";
pub const MERKLE_CRH_PERSONALIZATION: &str = "z.cash:Orchard-MerkleCRH";

type Hash = [u8; 32];

pub struct OrchardHasher {
    Q: Point,
}

impl OrchardHasher {
    pub fn new() -> Self {
        let Q: Point =
            Point::hash_to_curve(Q_PERSONALIZATION)(MERKLE_CRH_PERSONALIZATION.as_bytes());
        OrchardHasher { Q }
    }

    pub fn hash_combine(&self, depth: u8, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        log::info!("{} + {}", hex::encode(left), hex::encode(right));
        let mut acc = self.Q;
        let (S_x, S_y) = SINSEMILLA_S[depth as usize];
        let S_chunk = Affine::from_xy(S_x, S_y).unwrap();
        acc = (acc + S_chunk) + acc; // TODO Bail if + gives point at infinity?

        // Shift right by 1 bit and overwrite the 256th bit of left
        let mut left = left.clone();
        let mut right = right.clone();
        left[31] |= (right[0] & 1) << 7; // move the first bit of right into 256th of left
        for i in 0..32 {
            // move by 1 bit to fill the missing 256th bit of left
            let carry = if i < 31 { (right[i + 1] & 1) << 7 } else { 0 };
            right[i] = right[i] >> 1 | carry;
        }

        // we have 255*2/10 = 51 chunks
        let mut bit_offset = 0;
        let mut byte_offset = 0;
        for _ in 0..51 {
            let mut v = if byte_offset < 31 {
                left[byte_offset] as u16 | (left[byte_offset + 1] as u16) << 8
            } else if byte_offset == 31 {
                left[31] as u16 | (right[0] as u16) << 8
            } else {
                right[byte_offset - 32] as u16 | (right[byte_offset - 31] as u16) << 8
            };
            v = v >> bit_offset & 0x03FF; // keep 10 bits
            let (S_x, S_y) = SINSEMILLA_S[v as usize];
            let S_chunk = Affine::from_xy(S_x, S_y).unwrap();
            acc = (acc + S_chunk) + acc;
            bit_offset += 10;
            if bit_offset >= 8 {
                byte_offset += bit_offset / 8;
                bit_offset %= 8;
            }
        }

        let p = acc
            .to_affine()
            .coordinates()
            .map(|c| *c.x())
            .unwrap_or_else(pallas::Base::zero);
        p.to_repr()
    }

    pub fn empty_roots(&self, height: usize) -> Vec<OrchardHash> {
        let mut roots = vec![];
        let mut cur = OrchardHash(pallas::Base::from(2).to_repr());
        roots.push(cur);
        for depth in 0..height {
            cur = OrchardHash(self.hash_combine(depth as u8, &cur.0, &cur.0));
            roots.push(cur);
        }
        roots
    }
}
