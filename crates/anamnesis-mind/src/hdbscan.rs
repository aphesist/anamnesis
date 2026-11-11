//! HDBSCAN over a kNN graph.
//!
//! Classic HDBSCAN (Campello, Moulavi & Sander 2013; McInnes & Healy 2017)
//! builds the minimum spanning tree of the *mutual reachability* graph over
//! all pairs. Here the MST is built over the kNN graph only (an
//! approximation that is exact whenever the true MST edges are kNN edges,
//! which is the common case for k ≥ min_samples), and disconnected kNN
//! components are joined by edges heavier than any intra-component edge. The
//! rest — single-linkage tree, condensed tree, excess-of-mass selection — is
//! the standard algorithm.

use crate::knn::Knn;
use crate::linalg::Rows;

#[derive(Debug, Clone, Copy)]
pub struct HdbscanParams {
    pub min_cluster_size: usize,
    pub min_samples: usize,
    /// If nothing splits, return a single cluster instead of all-noise.
    pub allow_single_cluster: bool,
}

/// Union-find with path halving.
struct Dsu {
    parent: Vec<usize>,
}

impl Dsu {
    fn new(n: usize) -> Self {
        Dsu { parent: (0..n).collect() }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
}

/// Cluster labels (−1 = noise) for the points behind `knn`. `x` is the space
/// the kNN was computed in; it is only used to join disconnected components.
pub fn hdbscan(x: &Rows, knn: &Knn, params: HdbscanParams) -> Vec<i32> {
    let n = knn.n;
    if n == 0 {
        return vec![];
    }
    let mcs = params.min_cluster_size.max(2);
    if n < mcs || knn.k == 0 {
        return vec![if params.allow_single_cluster { 0 } else { -1 }; n];
    }
    let ms = params.min_samples.clamp(1, knn.k);
    let core: Vec<f32> = (0..n).map(|i| knn.distances(i)[ms - 1]).collect();

    // 1. mutual-reachability edges on the kNN graph
    let mut edges: Vec<(f32, u32, u32)> = Vec::with_capacity(n * knn.k);
    for i in 0..n {
        for (&j, &d) in knn.neighbors(i).iter().zip(knn.distances(i)) {
            let j = j as usize;
            let (a, b) = if i < j { (i, j) } else { (j, i) };
            let w = d.max(core[i]).max(core[j]);
            edges.push((w, a as u32, b as u32));
        }
    }
    edges.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    edges.dedup_by(|a, b| a.1 == b.1 && a.2 == b.2);

    // 2. Kruskal MST
    let mut dsu = Dsu::new(n);
    let mut mst: Vec<(f32, usize, usize)> = Vec::with_capacity(n - 1);
    let mut max_w = 0f32;
    for &(w, a, b) in &edges {
        let (ra, rb) = (dsu.find(a as usize), dsu.find(b as usize));
        if ra != rb {
            dsu.parent[ra] = rb;
            mst.push((w, a as usize, b as usize));
            max_w = max_w.max(w);
        }
    }
    // 3. join disconnected components through their centroids
    if mst.len() < n - 1 {
        let mut comp_of = vec![usize::MAX; n];
        let mut reps: Vec<usize> = vec![];
        let mut comp_index = std::collections::HashMap::new();
        for (i, c) in comp_of.iter_mut().enumerate() {
            let r = dsu.find(i);
            let len = comp_index.len();
            let ci = *comp_index.entry(r).or_insert_with(|| {
                reps.push(i);
                len
            });
            *c = ci;
        }
        let nc = reps.len();
        let mut cents = vec![vec![0f32; x.d]; nc];
        let mut counts = vec![0usize; nc];
        for i in 0..n {
            counts[comp_of[i]] += 1;
            for (a, b) in cents[comp_of[i]].iter_mut().zip(x.row(i)) {
                *a += b;
            }
        }
        for (c, &m) in cents.iter_mut().zip(&counts) {
            for v in c.iter_mut() {
                *v /= m as f32;
            }
        }
        let mut cedges = Vec::with_capacity(nc * (nc - 1) / 2);
        for a in 0..nc {
            for b in a + 1..nc {
                let d = crate::linalg::sq_dist(&cents[a], &cents[b]).sqrt();
                cedges.push((max_w + d + 1e-6, a, b));
            }
        }
        cedges.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut cd = Dsu::new(nc);
        for (w, a, b) in cedges {
            let (ra, rb) = (cd.find(a), cd.find(b));
            if ra != rb {
                cd.parent[ra] = rb;
                mst.push((w, reps[a], reps[b]));
            }
        }
        mst.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    }

    // 4. single-linkage tree: nodes 0..n are points, n.. internal
    let total = 2 * n - 1;
    let mut children = vec![(usize::MAX, usize::MAX); total];
    let mut ndist = vec![0f32; total];
    let mut size = vec![1usize; total];
    let mut dsu = Dsu::new(total);
    let mut next = n;
    for &(w, a, b) in &mst {
        let (ra, rb) = (dsu.find(a), dsu.find(b));
        if ra == rb {
            continue;
        }
        children[next] = (ra, rb);
        ndist[next] = w;
        size[next] = size[ra] + size[rb];
        dsu.parent[ra] = next;
        dsu.parent[rb] = next;
        next += 1;
    }
    let root = next - 1;

    // 5. condensed tree
    // cluster ids: 0 = root; entries (parent cluster, child, lambda, size, is_cluster)
    let lambda_of = |d: f32| 1.0 / (d as f64).max(1e-9);
    let mut node_cluster = vec![usize::MAX; total];
    node_cluster[root] = 0;
    let mut cluster_parent: Vec<usize> = vec![usize::MAX];
    let mut cluster_birth: Vec<f64> = vec![0.0];
    let mut cluster_children: Vec<Vec<usize>> = vec![vec![]];
    // points falling out: (point, cluster, lambda)
    let mut fall: Vec<(usize, usize, f64)> = Vec::with_capacity(n);
    // cluster-children entries contribute to stability via (lambda_birth_child - lambda_birth_parent) * size
    let mut stability: Vec<f64> = vec![0.0];

    let leaves_under = |node: usize, out: &mut Vec<usize>| {
        let mut stack = vec![node];
        while let Some(v) = stack.pop() {
            if v < n {
                out.push(v);
            } else {
                stack.push(children[v].0);
                stack.push(children[v].1);
            }
        }
    };

    let mut queue = std::collections::VecDeque::new();
    queue.push_back(root);
    let mut tmp = Vec::new();
    while let Some(node) = queue.pop_front() {
        let c = node_cluster[node];
        if node < n {
            // a single point carried as the cluster's own node: it falls out at lambda = inf-ish
            fall.push((node, c, lambda_of(0.0).min(cluster_birth[c].max(1.0) * 1e6)));
            continue;
        }
        let (l, r) = children[node];
        let lam = lambda_of(ndist[node]);
        let (ls, rs) = (size[l], size[r]);
        if ls >= mcs && rs >= mcs {
            for ch in [l, r] {
                let id = cluster_parent.len();
                cluster_parent.push(c);
                cluster_birth.push(lam);
                cluster_children.push(vec![]);
                stability.push(0.0);
                cluster_children[c].push(id);
                stability[c] += (lam - cluster_birth[c]) * size[ch] as f64;
                node_cluster[ch] = id;
                queue.push_back(ch);
            }
        } else {
            for (ch, chs) in [(l, ls), (r, rs)] {
                if chs >= mcs {
                    node_cluster[ch] = c;
                    queue.push_back(ch);
                } else {
                    tmp.clear();
                    leaves_under(ch, &mut tmp);
                    for &p in &tmp {
                        fall.push((p, c, lam));
                        stability[c] += lam - cluster_birth[c];
                    }
                }
            }
        }
    }

    // 6. excess of mass selection (bottom-up; children have larger ids)
    let nclusters = cluster_parent.len();
    let mut selected = vec![false; nclusters];
    let mut subtree = vec![0f64; nclusters];
    for c in (1..nclusters).rev() {
        let child_sum: f64 = cluster_children[c].iter().map(|&k| subtree[k]).sum();
        if cluster_children[c].is_empty() || stability[c] >= child_sum {
            selected[c] = true;
            subtree[c] = stability[c];
            // deselect descendants
            let mut st = cluster_children[c].clone();
            while let Some(k) = st.pop() {
                selected[k] = false;
                st.extend(cluster_children[k].iter().copied());
            }
        } else {
            subtree[c] = child_sum;
        }
    }

    // 7. labels
    let mut label_of_cluster = vec![-1i32; nclusters];
    let mut next_label = 0;
    for c in 1..nclusters {
        if selected[c] {
            label_of_cluster[c] = next_label;
            next_label += 1;
        }
    }
    if next_label == 0 {
        return vec![if params.allow_single_cluster { 0 } else { -1 }; n];
    }
    let mut labels = vec![-1i32; n];
    for &(p, c, _) in &fall {
        let mut k = c;
        while k != usize::MAX && k != 0 {
            if selected[k] {
                labels[p] = label_of_cluster[k];
                break;
            }
            k = cluster_parent[k];
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knn::knn_graph;
    use crate::rng::Rng;

    #[test]
    fn finds_three_blobs() {
        let mut r = Rng::new(11);
        let centers = [[0.0f32, 0.0], [10.0, 0.0], [0.0, 10.0]];
        let mut vs = vec![];
        for c in centers {
            for _ in 0..80 {
                vs.push(vec![c[0] + r.normal() as f32, c[1] + r.normal() as f32]);
            }
        }
        let x = Rows::from_vecs(&vs);
        let g = knn_graph(&x, 15);
        let labels = hdbscan(&x, &g, HdbscanParams { min_cluster_size: 15, min_samples: 5, allow_single_cluster: true });
        let distinct: std::collections::BTreeSet<i32> = labels.iter().copied().filter(|&l| l >= 0).collect();
        assert_eq!(distinct.len(), 3, "{distinct:?}");
        for b in 0..3 {
            let ls = &labels[b * 80..(b + 1) * 80];
            let main = ls.iter().filter(|&&l| l >= 0).copied().max_by_key(|l| ls.iter().filter(|&&m| m == *l).count()).unwrap();
            let agree = ls.iter().filter(|&&l| l == main).count();
            assert!(agree >= 70, "blob {b}: {agree}");
        }
    }
}
