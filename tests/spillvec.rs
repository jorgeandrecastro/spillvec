use std::fmt::Debug;

use spillvec::{DecodeError, Spill, SpillConfig, SpillVec, take};

fn small() -> SpillConfig {
    SpillConfig::new().max_items(10).batch_items(7)
}

fn collect<T: Spill>(v: SpillVec<T>) -> Vec<T> {
    v.into_iter().collect::<Result<Vec<_>, _>>().unwrap()
}

#[test]
fn stays_in_memory_below_threshold() {
    let mut v = SpillVec::with_config(small());
    v.consume(0..10u32).unwrap();
    assert!(!v.is_spilled());
    assert_eq!(v.len(), 10);
    assert_eq!(v.metrics().batches_written, 0);
    assert_eq!(collect(v), (0..10).collect::<Vec<_>>());
}

#[test]
fn spills_after_max_items() {
    let mut v = SpillVec::with_config(small());
    v.consume(0..1000u32).unwrap();
    assert!(v.is_spilled());
    let m = v.metrics();
    assert_eq!(m.items, 1000);
    assert!(m.batches_written > 0);
    assert!(m.bytes_written > 0);
    assert!(m.items_in_memory < 7 + 1);
    assert_eq!(collect(v), (0..1000).collect::<Vec<_>>());
}

#[test]
fn spills_after_max_bytes() {
    let cfg = SpillConfig::new().max_bytes(1000).batch_items(4);
    let mut v = SpillVec::with_config(cfg);
    let items: Vec<String> = (0..50).map(|i| format!("{i:0>100}")).collect();
    for s in &items {
        v.push(s.clone()).unwrap();
    }
    assert!(v.is_spilled());
    assert_eq!(collect(v), items);
}

#[test]
fn zero_threshold_spills_immediately() {
    let mut v = SpillVec::with_config(SpillConfig::new().max_items(0));
    v.push(1u8).unwrap();
    assert!(v.is_spilled());
    assert_eq!(collect(v), vec![1u8]);
}

#[test]
fn order_preserved_across_batches_and_tail() {
    let cfg = SpillConfig::new().max_items(3).batch_items(100);
    let mut v = SpillVec::with_config(cfg);
    v.consume(0..1234u64).unwrap();
    assert_eq!(collect(v), (0..1234).collect::<Vec<_>>());
}

#[test]
fn for_each_is_repeatable_and_push_continues() {
    let mut v = SpillVec::with_config(small());
    v.consume(0..50u32).unwrap();

    let mut first = Vec::new();
    v.for_each(|x| first.push(*x)).unwrap();
    assert_eq!(first, (0..50).collect::<Vec<_>>());

    v.consume(50..100u32).unwrap();

    let mut second = Vec::new();
    v.for_each(|x| second.push(*x)).unwrap();
    assert_eq!(second, (0..100).collect::<Vec<_>>());

    assert_eq!(collect(v), (0..100).collect::<Vec<_>>());
}

#[test]
fn complex_types_roundtrip() {
    type Row = (u32, String, Option<f64>, Vec<i16>, [u8; 3], bool, char);
    let rows: Vec<Row> = (0..200u32)
        .map(|i| {
            (
                i,
                format!("ligne {i} 你好 é"),
                if i % 3 == 0 { None } else { Some(i as f64 / 7.0) },
                (0..(i % 5) as i16).collect(),
                [i as u8, 1, 2],
                i % 2 == 0,
                char::from_u32(0x4E00 + i).unwrap(),
            )
        })
        .collect();

    let mut v = SpillVec::with_config(small());
    for r in &rows {
        v.push(r.clone()).unwrap();
    }
    assert!(v.is_spilled());
    assert_eq!(collect(v), rows);
}

#[derive(Debug, PartialEq, Clone)]
struct Point {
    x: i32,
    y: i32,
    label: String,
}

impl Spill for Point {
    fn encode(&self, out: &mut Vec<u8>) {
        self.x.encode(out);
        self.y.encode(out);
        self.label.encode(out);
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        Ok(Point { x: i32::decode(input)?, y: i32::decode(input)?, label: String::decode(input)? })
    }
    fn mem_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.label.len()
    }
}

#[test]
fn custom_type() {
    let pts: Vec<Point> = (0..100).map(|i| Point { x: i, y: -i, label: format!("p{i}") }).collect();
    let mut v = SpillVec::with_config(small());
    v.consume(pts.clone()).unwrap();
    assert!(v.is_spilled());
    assert_eq!(collect(v), pts);
}

#[test]
fn custom_decode_with_take() {
    struct Pair(u8, u8);
    impl Spill for Pair {
        fn encode(&self, out: &mut Vec<u8>) {
            out.push(self.0);
            out.push(self.1);
        }
        fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
            let b = take(input, 2)?;
            Ok(Pair(b[0], b[1]))
        }
    }
    let mut v = SpillVec::with_config(SpillConfig::new().max_items(1));
    v.consume((0..20u8).map(|i| Pair(i, i * 2))).unwrap();
    let out = collect(v);
    assert_eq!(out.len(), 20);
    assert!(out.iter().enumerate().all(|(i, p)| p.0 == i as u8 && p.1 == (i as u8) * 2));
}

#[test]
fn clear_resets_everything() {
    let mut v = SpillVec::with_config(small());
    v.consume(0..100u32).unwrap();
    assert!(v.is_spilled());
    v.clear();
    assert!(v.is_empty());
    assert!(!v.is_spilled());
    assert_eq!(v.metrics().bytes_written, 0);
    v.push(7).unwrap();
    assert_eq!(collect(v), vec![7u32]);
}

#[test]
fn empty() {
    let v: SpillVec<String> = SpillVec::new();
    assert!(v.is_empty());
    assert_eq!(v.into_iter().count(), 0);
}

#[test]
fn size_hint_is_exact() {
    let mut v = SpillVec::with_config(small());
    v.consume(0..100u32).unwrap();
    let mut it = v.into_iter();
    assert_eq!(it.size_hint(), (100, Some(100)));
    it.next();
    it.next();
    assert_eq!(it.size_hint(), (98, Some(98)));
}

#[test]
fn moves_across_threads() {
    let handles: Vec<_> = (0..4u32)
        .map(|t| {
            std::thread::spawn(move || {
                let mut v = SpillVec::with_config(small());
                v.consume((0..500).map(|i| t * 1000 + i)).unwrap();
                v
            })
        })
        .collect();
    for (t, h) in handles.into_iter().enumerate() {
        let v = h.join().unwrap();
        let expected: Vec<u32> = (0..500).map(|i| t as u32 * 1000 + i).collect();
        assert_eq!(collect(v), expected);
    }
}

#[cfg(unix)]
#[test]
fn no_file_left_on_disk() {
    let dir = std::env::temp_dir().join(format!("spillvec-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut v = SpillVec::with_config(small().temp_dir(&dir));
    v.consume(0..100u32).unwrap();
    assert!(v.is_spilled());
    // Fichier anonyme : rien de visible pendant l'utilisation…
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    assert_eq!(collect(v), (0..100).collect::<Vec<_>>());
    // … ni après.
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unwritable_temp_dir_keeps_item_in_memory() {
    let cfg = SpillConfig::new().max_items(0).temp_dir("/nonexistent/spillvec/dir");
    let mut v = SpillVec::with_config(cfg);
    assert!(v.push(42u32).is_err());
    assert_eq!(v.len(), 1);
    assert!(!v.is_spilled());
    let mut seen = Vec::new();
    v.for_each(|x| seen.push(*x)).unwrap();
    assert_eq!(seen, vec![42]);
}

// ---------- codec ----------

fn rt<T: Spill + PartialEq + Debug>(v: T) {
    let mut b = Vec::new();
    v.encode(&mut b);
    let mut s = &b[..];
    assert_eq!(T::decode(&mut s).unwrap(), v);
    assert!(s.is_empty(), "octets restants pour {v:?}");
}

#[test]
fn codec_roundtrips() {
    rt(0u8);
    rt(u16::MAX);
    rt(u32::MAX);
    rt(u64::MAX);
    rt(u128::MAX);
    rt(i8::MIN);
    rt(i16::MIN);
    rt(i32::MIN);
    rt(i64::MIN);
    rt(i128::MIN);
    rt(usize::MAX);
    rt(isize::MIN);
    rt(1.5f32);
    rt(-2.25f64);
    rt(true);
    rt('字');
    rt(());
    rt(String::from("héllo 你好 🙂"));
    rt(Some(Box::new(vec![Some(1u8), None])));
    rt([1u16, 2, 3, 4]);
    rt((1u8, "a".to_string(), 3i64, 'x', false, 9u16));
    rt(Vec::<String>::new());
}

#[test]
fn codec_rejects_bad_input() {
    let mut s: &[u8] = &[1, 2];
    assert_eq!(u32::decode(&mut s), Err(DecodeError::UnexpectedEof));

    let mut s: &[u8] = &[2];
    assert!(matches!(bool::decode(&mut s), Err(DecodeError::InvalidData(_))));

    let mut s: &[u8] = &[5];
    assert!(matches!(Option::<u8>::decode(&mut s), Err(DecodeError::InvalidData(_))));

    // String de 2 octets d'UTF-8 invalide
    let mut b = Vec::new();
    2usize.encode(&mut b);
    b.extend_from_slice(&[0xFF, 0xFE]);
    let mut s = &b[..];
    assert!(matches!(String::decode(&mut s), Err(DecodeError::InvalidData(_))));

    // char invalide (surrogate)
    let mut b = Vec::new();
    0xD800u32.encode(&mut b);
    let mut s = &b[..];
    assert!(matches!(char::decode(&mut s), Err(DecodeError::InvalidData(_))));

    // longueur de Vec démesurée : pas d'allocation géante, simple erreur
    let mut b = Vec::new();
    u64::MAX.encode(&mut b);
    let mut s = &b[..];
    assert!(Vec::<u32>::decode(&mut s).is_err());
}

#[test]
fn mem_size_counts_heap() {
    let s = "x".repeat(1000);
    assert!(s.mem_size() >= 1000);
    assert!(vec![s.clone(), s].mem_size() >= 2000);
    assert_eq!(7u32.mem_size(), 4);
}
