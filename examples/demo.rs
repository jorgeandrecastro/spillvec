//! Démo : `cargo run --release --example demo`
//!
//! Insère 200 000 lignes avec un seuil bas, affiche les métriques, puis relit tout.

use spillvec::{SpillConfig, SpillVec};

fn main() -> Result<(), spillvec::SpillError> {
    let cfg = SpillConfig::new()
        .max_items(10_000) // déborde sur disque au-delà de 10 000 éléments
        .batch_items(2_000); // lots de 2 000 éléments

    let mut lines: SpillVec<(u64, String)> = SpillVec::with_config(cfg);
    println!("avant : spilled = {}", lines.is_spilled());

    for i in 0..200_000u64 {
        lines.push((i, format!("ligne numéro {i}")))?;
    }

    let m = lines.metrics();
    println!("après : spilled = {}", lines.is_spilled());
    println!("  éléments           : {}", m.items);
    println!("  en mémoire         : {} (~{} octets)", m.items_in_memory, m.bytes_in_memory);
    println!("  lots écrits        : {}", m.batches_written);
    println!("  octets écrits      : {}", m.bytes_written);

    // Première passe sans consommer : validation à la volée.
    let mut longest = 0;
    lines.for_each(|(_, s)| longest = longest.max(s.len()))?;
    println!("ligne la plus longue : {longest} octets");

    // Passe finale en consommant (le fichier temporaire est supprimé à la fin).
    let mut sum = 0u64;
    for row in lines {
        let (i, _) = row?;
        sum += i;
    }
    println!("somme des indices : {sum}");
    Ok(())
}
