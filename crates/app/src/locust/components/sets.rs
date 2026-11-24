//! Sets: lists and trees.

use std::collections::{BTreeMap, HashSet};

use super::super::data::{as_int, as_number, truthy, DataTree, Kind, Value};
use super::*;

fn per_branch(t: &DataTree, f: impl Fn(&[Value]) -> Vec<Value>) -> DataTree {
    DataTree::from_branches(t.paths().into_iter().map(|p| {
        let b = f(t.branch_ref(&p));
        (p, b)
    }))
}

fn flags(t: &DataTree) -> Vec<bool> {
    t.items().iter().map(truthy).collect()
}

/// Items with duplicates (by repr) removed, keeping the first.
fn unique(items: impl IntoIterator<Item = Value>, keep: impl Fn(&str) -> bool) -> Vec<Value> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|v| {
        let k = v.repr();
        keep(&k) && seen.insert(k)
    }).collect()
}

pub fn register(r: &mut Registry) {
    r.tree("Series", "Sets", vec![num("Start", 0.0), num("Step", 1.0), int("Count", 5)], &["Series"], |_, t| {
        let (s, d, c) = (first_num(&t[0], 0.0), first_num(&t[1], 1.0), first_int(&t[2], 0).max(0));
        t1(DataTree::from_list((0..c).map(|i| n(s + d * i as f64)).collect()))
    })
    .describe("Numbers from Start, Count of them, Step apart");
    r.tree("Range", "Sets", vec![num("Start", 0.0), num("End", 1.0), int("Count", 5)], &["Range"], |_, t| {
        let (s, e, c) = (first_num(&t[0], 0.0), first_num(&t[1], 1.0), first_int(&t[2], 5).max(1));
        let step = if c > 1 { (e - s) / (c - 1) as f64 } else { 0.0 };
        t1(DataTree::from_list((0..c).map(|i| n(s + step * i as f64)).collect()))
    })
    .describe("Count numbers evenly from Start to End");
    r.tree("Flatten", "Sets", vec![tree("Tree")], &["Tree"], |_, t| t1(t[0].flatten()));
    r.tree("Graft", "Sets", vec![tree("Tree")], &["Tree"], |_, t| t1(t[0].graft()));
    r.tree("Simplify", "Sets", vec![tree("Tree")], &["Tree"], |_, t| t1(t[0].simplify()));
    r.tree("List Length", "Sets", vec![tree("Tree")], &["Count"], |_, t| t1(DataTree::from_list(t[0].paths().iter().map(|p| Value::Int(t[0].branch_ref(p).len() as i64)).collect())));
    r.tree("List Item", "Sets", vec![tree("Tree"), tree_d("Index", Kind::Int, Value::Int(0))], &["Item"], |_, t| {
        let mut idx: Vec<i64> = t[1].items().iter().map(as_int).collect();
        if idx.is_empty() {
            idx.push(0);
        }
        let mut out: BTreeMap<Vec<i64>, Vec<Value>> = BTreeMap::new();
        for path in t[0].paths() {
            let b = t[0].branch_ref(&path);
            for &k in &idx {
                let j = if k >= 0 { k } else { b.len() as i64 + k };
                if j >= 0 && (j as usize) < b.len() {
                    out.entry(path.clone()).or_default().push(b[j as usize].clone());
                }
            }
        }
        t1(DataTree { branches: out })
    })
    .describe("Pick items by index, per branch");
    r.tree("Reverse", "Sets", vec![tree("Tree")], &["Tree"], |_, t| t1(per_branch(&t[0], |b| b.iter().rev().cloned().collect())));
    r.tree("Reverse List", "Sets", vec![tree("Tree")], &["Tree"], |_, t| t1(per_branch(&t[0], |b| b.iter().rev().cloned().collect())));
    r.tree("Shift", "Sets", vec![tree("Tree"), int("Offset", 1), boolean("Wrap", false)], &["Tree"], |_, t| {
        let off = first_int(&t[1], 1);
        let wrap = first_bool(&t[2], false);
        t1(per_branch(&t[0], |b| {
            if b.is_empty() {
                return Vec::new();
            }
            let len = b.len() as i64;
            if wrap {
                let k = off.rem_euclid(len) as usize;
                b[k..].iter().chain(b[..k].iter()).cloned().collect()
            } else if off >= 0 {
                b[(off.min(len)) as usize..].to_vec()
            } else {
                b[..(len + off).max(0) as usize].to_vec()
            }
        }))
    });
    r.tree("Cull Pattern", "Sets", vec![tree("Tree"), tree_d("Pattern", Kind::Bool, Value::Bool(true))], &["Tree"], |_, t| {
        let f = flags(&t[1]);
        if f.is_empty() {
            return t1(t[0].clone());
        }
        t1(per_branch(&t[0], |b| b.iter().enumerate().filter(|(i, _)| f[i % f.len()]).map(|(_, v)| v.clone()).collect()))
    })
    .describe("Keep items where the pattern is true");
    r.tree("Merge", "Sets", vec![tree("A"), tree("B")], &["Tree"], |_, t| t1(t[0].merge(&t[1])));
    r.tree("Sort", "Sets", vec![tree("Tree")], &["Tree"], |_, t| {
        t1(per_branch(&t[0], |b| {
            let mut v = b.to_vec();
            v.sort_by(|x, y| as_number(x).total_cmp(&as_number(y)));
            v
        }))
    });
    r.tree("Partition", "Sets", vec![tree("Tree"), int("Size", 2), int("Step", 2)], &["Tree"], |_, t| {
        let s = first_int(&t[1], 2).max(1);
        let st = first_int(&t[2], s).max(1);
        t1(t[0].partition(s, Some(st)))
    });
    r.tree("Graft By", "Sets", vec![tree("Tree"), int("Count", 2)], &["Tree"], |_, t| t1(t[0].graft_by(first_int(&t[1], 2).max(1))));
    r.tree("Trim Tree", "Sets", vec![tree("Tree"), int("Depth", 1)], &["Tree"], |_, t| t1(t[0].trim(first_int(&t[1], 1))));
    r.tree("Shift Paths", "Sets", vec![tree("Tree"), int("Offset", 1)], &["Tree"], |_, t| t1(t[0].shift_paths(first_int(&t[1], 1))));
    r.tree("Deconstruct Path", "Sets", vec![tree("Tree")], &["Paths"], |_, t| t1(t[0].branch_paths()));
    r.tree("Cull Index", "Sets", vec![tree("Tree"), tree_d("Index", Kind::Int, Value::Int(0))], &["Tree"], |_, t| {
        let idx: HashSet<i64> = t[1].items().iter().map(as_int).collect();
        t1(per_branch(&t[0], |b| b.iter().enumerate().filter(|(i, _)| !idx.contains(&(*i as i64))).map(|(_, v)| v.clone()).collect()))
    });
    r.tree("Replace Items", "Sets", vec![tree("Tree"), tree_d("Index", Kind::Int, Value::Int(0)), any_d("Item", n(0.0))], &["Tree"], |_, t| {
        let mut idx: Vec<i64> = t[1].items().iter().map(as_int).collect();
        if idx.is_empty() {
            idx.push(0);
        }
        let mut repl = t[2].items();
        if repl.is_empty() {
            repl.push(Value::Null);
        }
        t1(per_branch(&t[0], |b| {
            let mut v = b.to_vec();
            for (k, &i) in idx.iter().enumerate() {
                let j = if i >= 0 { i } else { v.len() as i64 + i };
                if j >= 0 && (j as usize) < v.len() {
                    v[j as usize] = repl[k % repl.len()].clone();
                }
            }
            v
        }))
    });
    r.tree("Insert Items", "Sets", vec![tree("Tree"), tree("Item"), int("Index", 0)], &["Tree"], |_, t| {
        let k = first_int(&t[2], 0);
        let items = t[1].items();
        t1(per_branch(&t[0], |b| {
            let pos = k.clamp(0, b.len() as i64) as usize;
            let mut v = b[..pos].to_vec();
            v.extend(items.iter().cloned());
            v.extend(b[pos..].iter().cloned());
            v
        }))
    });
    r.tree("Dispatch", "Sets", vec![tree("Tree"), tree_d("Pattern", Kind::Bool, Value::Bool(true))], &["A", "B"], |_, t| {
        let mut f = flags(&t[1]);
        if f.is_empty() {
            f.push(true);
        }
        let a = per_branch(&t[0], |b| b.iter().enumerate().filter(|(i, _)| f[i % f.len()]).map(|(_, v)| v.clone()).collect());
        let bb = per_branch(&t[0], |b| b.iter().enumerate().filter(|(i, _)| !f[i % f.len()]).map(|(_, v)| v.clone()).collect());
        vec![a, bb]
    });
    r.tree("Weave", "Sets", vec![tree("A"), tree("B"), tree_d("Pattern", Kind::Bool, Value::Bool(true))], &["Tree"], |_, t| {
        let (a, b) = (t[0].items(), t[1].items());
        let mut f = flags(&t[2]);
        if f.is_empty() {
            f.push(true);
        }
        let (mut ia, mut ib, mut i) = (0, 0, 0);
        let mut out = Vec::new();
        while ia < a.len() || ib < b.len() {
            let take_a = f[i % f.len()];
            if (take_a && ia < a.len()) || (!take_a && ib >= b.len()) {
                out.push(a[ia].clone());
                ia += 1;
            } else {
                out.push(b[ib].clone());
                ib += 1;
            }
            i += 1;
        }
        t1(DataTree::from_list(out))
    });
    r.tree("Entwine", "Sets", vec![tree("A"), tree("B"), tree("C").opt()], &["Tree"], |_, t| {
        let mut out: BTreeMap<Vec<i64>, Vec<Value>> = BTreeMap::new();
        for (i, tr) in t.iter().enumerate() {
            for path in tr.paths() {
                let mut p = path.clone();
                p.push(i as i64);
                out.entry(p).or_default().extend(tr.branch(&path));
            }
        }
        t1(DataTree { branches: out })
    });
    r.tree("List Union", "Sets", vec![tree("A"), tree("B")], &["Tree"], |_, t| t1(DataTree::from_list(unique(t[0].items().into_iter().chain(t[1].items()), |_| true))));
    r.tree("List Intersection", "Sets", vec![tree("A"), tree("B")], &["Tree"], |_, t| {
        let keys: HashSet<String> = t[1].items().iter().map(|v| v.repr()).collect();
        t1(DataTree::from_list(unique(t[0].items(), |k| keys.contains(k))))
    });
    r.tree("List Difference", "Sets", vec![tree("A"), tree("B")], &["Tree"], |_, t| {
        let keys: HashSet<String> = t[1].items().iter().map(|v| v.repr()).collect();
        t1(DataTree::from_list(unique(t[0].items(), |k| !keys.contains(k))))
    });
    r.tree("Item Index", "Sets", vec![tree("Tree"), any_d("Item", n(0.0))], &["Index"], |_, t| {
        let target = t[1].first().map(|v| v.repr()).unwrap_or_else(|| "None".into());
        t1(DataTree::from_list(t[0].items().iter().enumerate().filter(|(_, v)| v.repr() == target).map(|(i, _)| Value::Int(i as i64)).collect()))
    });
    r.tree("Sub List", "Sets", vec![tree("Tree"), int("From", 0), int("Count", 2)], &["Tree"], |_, t| {
        let i = first_int(&t[1], 0).max(0) as usize;
        let c = first_int(&t[2], 2).max(0) as usize;
        t1(per_branch(&t[0], |b| b.iter().skip(i).take(c).cloned().collect()))
    });
    r.tree("Fibonacci", "Sets", vec![int("Count", 10)], &["Values"], |_, t| {
        let k = first_int(&t[0], 10).clamp(0, 90);
        let (mut a, mut b) = (0u64, 1u64);
        let mut out = Vec::new();
        for _ in 0..k {
            out.push(n(a as f64));
            let c = a + b;
            a = b;
            b = c;
        }
        t1(DataTree::from_list(out))
    });
}
