//! # spillvec
//!
//! Un `Vec` qui **déborde sur le disque** au-delà d'un seuil, sans aucune dépendance.
//!
//! Utile quand une file n'est pas adaptée : on collecte d'abord toutes les données, puis on les
//! consomme (plusieurs threads qui produisent lentement de gros vecteurs, envoi d'un CSV de
//! plusieurs Go que l'on valide ligne par ligne, etc.).
//!
//! * Le fichier temporaire n'est créé qu'**après** dépassement du seuil (éléments **ou** octets).
//! * Fichier **anonyme** : supprimé dès sa création sous Unix, `DELETE_ON_CLOSE` sous Windows.
//!   Rien ne reste sur le disque, même après un crash.
//! * Sérialisation maison via le trait [`Spill`] : pas de `serde`, pas de `Clone` requis.
//! * Chaque lot écrit est protégé par un **CRC32**.
//! * `SpillVec<T>` est `Send` dès que `T` l'est.
//!
//! ```
//! use spillvec::{SpillConfig, SpillVec};
//!
//! // Déborde sur disque au-delà de 4 éléments en mémoire.
//! let mut data = SpillVec::with_config(SpillConfig::new().max_items(4));
//! data.consume(0..9u32).unwrap();
//! assert!(data.is_spilled());
//!
//! for value in data {
//!     println!("lu : {}", value.unwrap());
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod codec;
mod crc;
mod error;
mod tempfile;
mod vec;

pub use codec::{DecodeError, Spill, take};
pub use error::SpillError;
pub use vec::{IntoIter, Metrics, SpillConfig, SpillVec};
