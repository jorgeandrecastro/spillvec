# spillvec 💾

[![Crates.io](https://img.shields.io/crates/v/spillvec.svg)](https://crates.io/crates/spillvec)
[![docs.rs](https://img.shields.io/docsrs/spillvec)](https://docs.rs/spillvec)
[![Téléchargements](https://img.shields.io/crates/d/spillvec.svg)](https://crates.io/crates/spillvec)
[![MSRV](https://img.shields.io/crates/msrv/spillvec.svg)](https://crates.io/crates/spillvec)
[![Licence](https://img.shields.io/crates/l/spillvec.svg)](https://crates.io/crates/spillvec)
[![Dépendances](https://img.shields.io/badge/dépendances-0-brightgreen.svg)](https://crates.io/crates/spillvec/dependencies)
[![unsafe interdit](https://img.shields.io/badge/unsafe-interdit-success.svg)](https://github.com/rust-secure-code/safety-dance/)
[![CI](https://github.com/jorgeandrecastro/spillvec/actions/workflows/ci.yml/badge.svg)](https://github.com/jorgeandrecastro/spillvec/actions)

Un `Vec` qui **déborde sur le disque** au-delà d'un seuil, **sans aucune dépendance**.

Utile quand une file (queue) ne convient pas et que l'on veut d'abord collecter toutes les données,
puis les consommer : plusieurs threads qui produisent lentement de gros vecteurs pour un seul
consommateur, ou l'envoi d'un CSV de plusieurs Go vers un serveur HTTP dont on valide chaque ligne
au fil de l'eau, sans ouvrir de transaction en base ni tout garder en mémoire.

```rust
use spillvec::{SpillConfig, SpillVec};

// Déborde sur disque au-delà de 4 éléments en mémoire.
let mut data = SpillVec::with_config(SpillConfig::new().max_items(4));
data.consume(0..9u32).unwrap();
assert!(data.is_spilled());

for value in data {
    println!("lu : {}", value.unwrap());
}
```

## Fonctionnalités

- **Zéro dépendance** : sérialisation, CRC32 et fichier temporaire sont faits maison (std uniquement).
- **Seuil en éléments *et* en octets** : `max_items` et `max_bytes`, le tas (`String`, `Vec`…) est compté
  via `Spill::mem_size`.
- **Fichier temporaire créé uniquement après dépassement du seuil.**
- **Fichier anonyme** : supprimé dès sa création sous Unix (permissions `0600`), `DELETE_ON_CLOSE`
  sous Windows. Rien ne reste sur le disque, même après un crash ou un `kill -9`.
- **Intégrité** : chaque lot écrit est protégé par un CRC32 (longueur, nombre d'éléments et données).
  Corruption ou troncature détectées et signalées par une erreur, jamais de données silencieusement fausses.
- **Pas de `serde`, pas de `Clone`** : un trait `Spill` minimal (`encode` / `decode`), déjà implémenté
  pour les entiers, flottants, `bool`, `char`, `()`, `String`, `Vec<T>`, `Option<T>`, `Box<T>`,
  `[T; N]` et les tuples jusqu'à 8 éléments.
- **Plusieurs lectures** : `for_each` parcourt sans consommer, autant de fois que voulu, et on peut
  continuer à `push` ensuite.
- **Métriques** : lots et octets écrits, éléments en mémoire (`metrics()`).
- **`Send`** : un `SpillVec<T>` peut passer d'un thread à l'autre dès que `T: Send`.
- **100 % safe** : `#![forbid(unsafe_code)]`.

## API

| Élément | Rôle |
|---|---|
| `SpillVec::new()` / `with_config(cfg)` | création |
| `push(item)` / `consume(iter)` | ajout (renvoie un `Result`, le disque peut échouer) |
| `for_each(\|x\| …)` | parcours sans consommer, répétable |
| `into_iter()` | consomme, `Item = Result<T, SpillError>`, supprime le fichier à la fin |
| `len()` / `is_empty()` / `is_spilled()` | état |
| `metrics()` | compteurs (`items`, `items_in_memory`, `batches_written`, `bytes_written`…) |
| `clear()` | remet à zéro et supprime le fichier |
| `SpillConfig` | `max_items`, `max_bytes`, `batch_items`, `temp_dir` |
| trait `Spill` | `encode`, `decode`, `mem_size` |

## Type personnalisé

```rust
use spillvec::{DecodeError, Spill};

struct Point { x: i32, y: i32, label: String }

impl Spill for Point {
    fn encode(&self, out: &mut Vec<u8>) {
        self.x.encode(out);
        self.y.encode(out);
        self.label.encode(out);
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        Ok(Point { x: i32::decode(input)?, y: i32::decode(input)?, label: String::decode(input)? })
    }
    // Recommandé si le type possède du tas : le seuil `max_bytes` en dépend.
    fn mem_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.label.len()
    }
}
```

## Exemple

```bash
cargo run --release --example demo
```

## Limites

- Presque toutes les opérations renvoient un `Result` : elles peuvent faire des E/S.
- `Compression` n'existe pas (pas de compression des lots pour l'instant).
- Pas de support async.
- Pas d'accès aléatoire (`v[i]`) : on écrit à la fin, on lit du début à la fin.
- Si un `push` échoue à cause du disque, l'élément reste conservé en mémoire : rien n'est perdu.
- Le chemin Windows (`DELETE_ON_CLOSE`) n'a pas pu être exécuté lors du développement initial :
  les retours sur cette plateforme sont les bienvenus.

## Licence

GPL-2.0-or-later.
