//! Synthetic data for tests: gaussian blobs on the unit sphere.
#![allow(dead_code)]

use anamnesis_core::Id;
use anamnesis_mind::rng::Rng;

pub fn unit(rng: &mut Rng, d: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..d).map(|_| rng.normal() as f32).collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
    v
}

pub fn around(rng: &mut Rng, c: &[f32], sigma: f32) -> Vec<f32> {
    let mut v: Vec<f32> = c.iter().map(|x| x + sigma * rng.normal() as f32).collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
    v
}

pub fn blob(rng: &mut Rng, c: &[f32], n: usize, sigma: f32) -> Vec<Vec<f32>> {
    (0..n).map(|_| around(rng, c, sigma)).collect()
}

pub fn ids(n: usize, prefix: &str) -> Vec<Id> {
    (0..n).map(|i| Id(format!("{prefix}{i:05}"))).collect()
}
