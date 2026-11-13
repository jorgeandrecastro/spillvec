use std::fmt;
use std::io;

use crate::codec::DecodeError;

/// Erreur de [`SpillVec`](crate::SpillVec).
#[derive(Debug)]
pub enum SpillError {
    /// Erreur d'entrée/sortie sur le fichier temporaire.
    Io(io::Error),
    /// Un élément n'a pas pu être décodé depuis le disque.
    Decode(DecodeError),
    /// Le fichier temporaire est corrompu (checksum invalide, fichier tronqué…).
    Corrupted(&'static str),
}

impl fmt::Display for SpillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpillError::Io(e) => write!(f, "erreur d'E/S : {e}"),
            SpillError::Decode(e) => write!(f, "décodage impossible : {e}"),
            SpillError::Corrupted(m) => write!(f, "fichier temporaire corrompu : {m}"),
        }
    }
}

impl std::error::Error for SpillError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SpillError::Io(e) => Some(e),
            SpillError::Decode(e) => Some(e),
            SpillError::Corrupted(_) => None,
        }
    }
}

impl From<io::Error> for SpillError {
    fn from(e: io::Error) -> Self {
        SpillError::Io(e)
    }
}

impl From<DecodeError> for SpillError {
    fn from(e: DecodeError) -> Self {
        SpillError::Decode(e)
    }
}
