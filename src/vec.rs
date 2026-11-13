use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use crate::codec::Spill;
use crate::crc::Crc32;
use crate::error::SpillError;
use crate::tempfile::TempFile;

/// Taille de l'en-tête d'un lot : longueur (u64) + nombre d'éléments (u32) + CRC32 (u32).
const HEADER_LEN: usize = 16;

/// Configuration d'un [`SpillVec`].
///
/// Le débordement sur disque se déclenche dès que **l'une** des deux limites est dépassée.
#[derive(Debug, Clone)]
pub struct SpillConfig {
    max_items: usize,
    max_bytes: usize,
    batch_items: usize,
    temp_dir: Option<PathBuf>,
}

impl Default for SpillConfig {
    fn default() -> Self {
        SpillConfig {
            max_items: usize::MAX,
            max_bytes: 64 * 1024 * 1024,
            batch_items: 1024,
            temp_dir: None,
        }
    }
}

impl SpillConfig {
    /// Configuration par défaut : 64 MiB en mémoire, pas de limite en nombre d'éléments.
    pub fn new() -> Self {
        Self::default()
    }

    /// Déborde sur disque au-delà de `n` éléments en mémoire.
    pub fn max_items(mut self, n: usize) -> Self {
        self.max_items = n;
        self
    }

    /// Déborde sur disque au-delà de `n` octets estimés en mémoire (voir [`Spill::mem_size`]).
    pub fn max_bytes(mut self, n: usize) -> Self {
        self.max_bytes = n;
        self
    }

    /// Nombre d'éléments par lot écrit sur disque (minimum 1).
    pub fn batch_items(mut self, n: usize) -> Self {
        self.batch_items = n.clamp(1, u32::MAX as usize);
        self
    }

    /// Dossier du fichier temporaire (par défaut : le dossier temporaire du système).
    pub fn temp_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.temp_dir = Some(dir.into());
        self
    }
}

/// Instantané des compteurs d'un [`SpillVec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Metrics {
    /// Nombre total d'éléments.
    pub items: usize,
    /// Éléments actuellement en mémoire.
    pub items_in_memory: usize,
    /// Taille estimée des éléments en mémoire.
    pub bytes_in_memory: usize,
    /// Lots écrits sur disque.
    pub batches_written: usize,
    /// Octets écrits sur disque (en-têtes compris).
    pub bytes_written: u64,
}

/// Un `Vec` qui déborde sur le disque au-delà d'un seuil.
///
/// Les éléments sont d'abord gardés en mémoire. Dès qu'un seuil de [`SpillConfig`] est dépassé,
/// un fichier temporaire anonyme est créé et les éléments y sont écrits par lots, chacun protégé
/// par un CRC32. L'ordre d'insertion est toujours conservé.
pub struct SpillVec<T> {
    cfg: SpillConfig,
    /// Éléments les plus récents, toujours *après* ceux du fichier.
    mem: Vec<T>,
    mem_bytes: usize,
    file: Option<TempFile>,
    len: usize,
    batches: usize,
    bytes_written: u64,
    scratch: Vec<u8>,
}

impl<T: Spill> Default for SpillVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Debug for SpillVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpillVec")
            .field("len", &self.len)
            .field("spilled", &self.file.is_some())
            .field("batches_written", &self.batches)
            .finish()
    }
}

impl<T: Spill> SpillVec<T> {
    /// Crée un `SpillVec` avec la configuration par défaut.
    pub fn new() -> Self {
        Self::with_config(SpillConfig::default())
    }

    /// Crée un `SpillVec` avec une configuration personnalisée.
    pub fn with_config(cfg: SpillConfig) -> Self {
        SpillVec {
            cfg,
            mem: Vec::new(),
            mem_bytes: 0,
            file: None,
            len: 0,
            batches: 0,
            bytes_written: 0,
            scratch: Vec::new(),
        }
    }

    /// Nombre total d'éléments (mémoire + disque).
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` si aucun élément n'a été ajouté.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `true` si un fichier temporaire a été créé.
    pub fn is_spilled(&self) -> bool {
        self.file.is_some()
    }

    /// Compteurs courants.
    pub fn metrics(&self) -> Metrics {
        Metrics {
            items: self.len,
            items_in_memory: self.mem.len(),
            bytes_in_memory: self.mem_bytes,
            batches_written: self.batches,
            bytes_written: self.bytes_written,
        }
    }

    /// Ajoute un élément.
    ///
    /// En cas d'erreur d'E/S, l'élément reste conservé en mémoire : rien n'est perdu.
    pub fn push(&mut self, item: T) -> Result<(), SpillError> {
        self.mem_bytes = self.mem_bytes.saturating_add(item.mem_size());
        self.mem.push(item);
        self.len += 1;

        if self.file.is_none() {
            if self.mem.len() > self.cfg.max_items || self.mem_bytes > self.cfg.max_bytes {
                self.file = Some(TempFile::create(self.cfg.temp_dir.as_deref())?);
                self.flush_mem()?;
            }
        } else if self.mem.len() >= self.cfg.batch_items || self.mem_bytes >= self.cfg.max_bytes {
            self.flush_mem()?;
        }
        Ok(())
    }

    /// Ajoute tous les éléments d'un itérateur.
    pub fn consume<I: IntoIterator<Item = T>>(&mut self, iter: I) -> Result<(), SpillError> {
        for item in iter {
            self.push(item)?;
        }
        Ok(())
    }

    /// Parcourt tous les éléments dans l'ordre d'insertion, sans consommer le `SpillVec`.
    ///
    /// Peut être appelé autant de fois que voulu, et on peut continuer à `push` ensuite.
    pub fn for_each<F: FnMut(&T)>(&mut self, mut f: F) -> Result<(), SpillError> {
        if let Some(tf) = self.file.as_mut() {
            let mut reader = BatchReader::new(self.batches, self.bytes_written);
            let mut items: Vec<T> = Vec::new();
            while reader.next_batch(&mut tf.file, &mut items)? {
                for item in &items {
                    f(item);
                }
                items.clear();
            }
        }
        for item in &self.mem {
            f(item);
        }
        Ok(())
    }

    /// Vide le `SpillVec` et supprime le fichier temporaire.
    pub fn clear(&mut self) {
        self.mem.clear();
        self.mem_bytes = 0;
        self.len = 0;
        self.batches = 0;
        self.bytes_written = 0;
        self.file = None;
    }

    /// Écrit tout le contenu de `mem` sur disque, par lots.
    fn flush_mem(&mut self) -> Result<(), SpillError> {
        let Some(tf) = self.file.as_mut() else {
            return Ok(());
        };
        let mut done = 0usize;
        let mut result = Ok(());
        for chunk in self.mem.chunks(self.cfg.batch_items) {
            match write_batch(&mut tf.file, &mut self.scratch, self.bytes_written, chunk) {
                Ok(written) => {
                    self.bytes_written += written;
                    self.batches += 1;
                    done += chunk.len();
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        // Seuls les lots réellement écrits quittent la mémoire.
        self.mem.drain(..done);
        self.mem_bytes = self.mem.iter().map(Spill::mem_size).sum();
        result
    }
}

/// Encode un lot (`en-tête + charge utile`) et l'écrit à `offset`.
fn write_batch<T: Spill>(
    file: &mut File,
    scratch: &mut Vec<u8>,
    offset: u64,
    chunk: &[T],
) -> Result<u64, SpillError> {
    scratch.clear();
    scratch.resize(HEADER_LEN, 0);
    for item in chunk {
        item.encode(scratch);
    }
    let payload_len = (scratch.len() - HEADER_LEN) as u64;
    scratch[0..8].copy_from_slice(&payload_len.to_le_bytes());
    scratch[8..12].copy_from_slice(&(chunk.len() as u32).to_le_bytes());

    // Le CRC couvre la longueur, le nombre d'éléments et la charge utile.
    let mut crc = Crc32::new();
    crc.update(&scratch[..12]);
    crc.update(&scratch[HEADER_LEN..]);
    scratch[12..16].copy_from_slice(&crc.finalize().to_le_bytes());

    // On écrit à l'offset connu (pas en fin de fichier) : un lot partiel
    // laissé par une écriture échouée est simplement écrasé.
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(scratch)?;
    Ok(scratch.len() as u64)
}

/// Lecture séquentielle des lots d'un fichier.
struct BatchReader {
    remaining: usize,
    pos: u64,
    limit: u64,
    seeked: bool,
    buf: Vec<u8>,
}

impl BatchReader {
    fn new(batches: usize, limit: u64) -> Self {
        BatchReader { remaining: batches, pos: 0, limit, seeked: false, buf: Vec::new() }
    }

    /// Lit et décode le lot suivant dans `out`. `Ok(false)` quand il n'y en a plus.
    fn next_batch<T: Spill>(&mut self, file: &mut File, out: &mut Vec<T>) -> Result<bool, SpillError> {
        if self.remaining == 0 {
            return Ok(false);
        }
        if !self.seeked {
            file.seek(SeekFrom::Start(0))?;
            self.seeked = true;
        }

        let mut header = [0u8; HEADER_LEN];
        read_exact(file, &mut header)?;
        let payload_len = u64::from_le_bytes(header[0..8].try_into().expect("8 octets"));
        let count = u32::from_le_bytes(header[8..12].try_into().expect("4 octets")) as usize;
        let expected_crc = u32::from_le_bytes(header[12..16].try_into().expect("4 octets"));

        // La longueur n'est pas encore vérifiée par le CRC : on la borne par la taille du fichier.
        let end = (self.pos + HEADER_LEN as u64)
            .checked_add(payload_len)
            .filter(|&e| e <= self.limit)
            .ok_or(SpillError::Corrupted("longueur de lot hors limites"))?;

        self.buf.clear();
        self.buf.resize(payload_len as usize, 0);
        read_exact(file, &mut self.buf)?;

        let mut crc = Crc32::new();
        crc.update(&header[..12]);
        crc.update(&self.buf);
        if crc.finalize() != expected_crc {
            return Err(SpillError::Corrupted("checksum invalide"));
        }

        let mut input = &self.buf[..];
        out.reserve(count.min(self.buf.len()));
        for _ in 0..count {
            out.push(T::decode(&mut input)?);
        }
        if !input.is_empty() {
            return Err(SpillError::Corrupted("octets en trop dans un lot"));
        }

        self.pos = end;
        self.remaining -= 1;
        Ok(true)
    }
}

fn read_exact(file: &mut File, buf: &mut [u8]) -> Result<(), SpillError> {
    file.read_exact(buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            SpillError::Corrupted("fichier tronqué")
        } else {
            SpillError::Io(e)
        }
    })
}

/// Itérateur consommant un [`SpillVec`], dans l'ordre d'insertion.
///
/// Chaque élément est un `Result` car la lecture du disque peut échouer.
/// Après une erreur, l'itérateur est terminé.
pub struct IntoIter<T> {
    file: Option<TempFile>,
    reader: BatchReader,
    current: std::vec::IntoIter<T>,
    tail: std::vec::IntoIter<T>,
    remaining: usize,
    failed: bool,
}

impl<T: Spill> Iterator for IntoIter<T> {
    type Item = Result<T, SpillError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(x) = self.current.next() {
                self.remaining -= 1;
                return Some(Ok(x));
            }
            if let Some(tf) = self.file.as_mut() {
                let mut batch = Vec::new();
                match self.reader.next_batch(&mut tf.file, &mut batch) {
                    Ok(true) => {
                        self.current = batch.into_iter();
                        continue;
                    }
                    Ok(false) => self.file = None, // ferme et supprime le fichier
                    Err(e) => {
                        self.failed = true;
                        self.remaining = 0;
                        return Some(Err(e));
                    }
                }
            }
            return match self.tail.next() {
                Some(x) => {
                    self.remaining -= 1;
                    Some(Ok(x))
                }
                None => None,
            };
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T: Spill> IntoIterator for SpillVec<T> {
    type Item = Result<T, SpillError>;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
        IntoIter {
            file: self.file,
            reader: BatchReader::new(self.batches, self.bytes_written),
            current: Vec::new().into_iter(),
            tail: self.mem.into_iter(),
            remaining: self.len,
            failed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spilled(n: u32) -> SpillVec<u32> {
        let mut v = SpillVec::with_config(SpillConfig::new().max_items(5).batch_items(10));
        v.consume(0..n).unwrap();
        assert!(v.is_spilled());
        v
    }

    #[test]
    fn detects_flipped_byte() {
        let mut v = spilled(100);
        let f = &mut v.file.as_mut().unwrap().file;
        f.seek(SeekFrom::Start(20)).unwrap();
        let mut b = [0u8; 1];
        f.read_exact(&mut b).unwrap();
        f.seek(SeekFrom::Start(20)).unwrap();
        f.write_all(&[b[0] ^ 0xFF]).unwrap();

        let res: Result<Vec<u32>, _> = v.into_iter().collect();
        assert!(matches!(res, Err(SpillError::Corrupted("checksum invalide"))));
    }

    #[test]
    fn detects_truncated_file() {
        let mut v = spilled(100);
        v.file.as_mut().unwrap().file.set_len(30).unwrap();
        let res: Result<Vec<u32>, _> = v.into_iter().collect();
        assert!(matches!(res, Err(SpillError::Corrupted(_))));
    }

    #[test]
    fn iterator_stops_after_error() {
        let mut v = spilled(100);
        v.file.as_mut().unwrap().file.set_len(30).unwrap();
        let mut it = v.into_iter();
        assert!(it.next().unwrap().is_err());
        assert!(it.next().is_none());
    }
}
