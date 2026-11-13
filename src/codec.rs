//! Sérialisation binaire maison (little-endian), sans `serde`.

use std::fmt;
use std::mem::size_of;

/// Erreur de décodage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Les données s'arrêtent avant la fin de l'élément.
    UnexpectedEof,
    /// Les données sont invalides (booléen hors 0/1, UTF-8 incorrect…).
    InvalidData(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::UnexpectedEof => f.write_str("fin de données inattendue"),
            DecodeError::InvalidData(m) => write!(f, "données invalides : {m}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Un type qui peut être stocké dans un [`SpillVec`](crate::SpillVec).
///
/// Aucun `serde`, aucun `Clone` requis. Implémenté pour les entiers, flottants, `bool`, `char`,
/// `()`, `String`, `Vec<T>`, `Option<T>`, `Box<T>`, `[T; N]` et les tuples jusqu'à 8 éléments.
///
/// ```
/// use spillvec::{DecodeError, Spill};
///
/// struct Point { x: i32, y: i32 }
///
/// impl Spill for Point {
///     fn encode(&self, out: &mut Vec<u8>) {
///         self.x.encode(out);
///         self.y.encode(out);
///     }
///     fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
///         Ok(Point { x: i32::decode(input)?, y: i32::decode(input)? })
///     }
/// }
/// ```
pub trait Spill: Sized {
    /// Ajoute la représentation binaire de `self` à `out`.
    fn encode(&self, out: &mut Vec<u8>);

    /// Lit un élément au début de `input` et avance `input` d'autant.
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError>;

    /// Taille estimée en mémoire, **tas compris** (sert au seuil `max_bytes`).
    ///
    /// Par défaut : `size_of::<Self>()`. À surcharger pour les types qui possèdent du tas.
    fn mem_size(&self) -> usize {
        size_of::<Self>()
    }
}

/// Retire et renvoie les `n` premiers octets de `input`.
///
/// Brique de base pour écrire un [`Spill::decode`] personnalisé.
pub fn take<'a>(input: &mut &'a [u8], n: usize) -> Result<&'a [u8], DecodeError> {
    if input.len() < n {
        return Err(DecodeError::UnexpectedEof);
    }
    let (head, tail) = input.split_at(n);
    *input = tail;
    Ok(head)
}

macro_rules! impl_num {
    ($($t:ty),* $(,)?) => {$(
        impl Spill for $t {
            #[inline]
            fn encode(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
            #[inline]
            fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
                let bytes = take(input, size_of::<$t>())?;
                let mut a = [0u8; size_of::<$t>()];
                a.copy_from_slice(bytes);
                Ok(<$t>::from_le_bytes(a))
            }
        }
    )*};
}
impl_num!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

impl Spill for usize {
    fn encode(&self, out: &mut Vec<u8>) {
        (*self as u64).encode(out);
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        usize::try_from(u64::decode(input)?).map_err(|_| DecodeError::InvalidData("usize trop grand"))
    }
}

impl Spill for isize {
    fn encode(&self, out: &mut Vec<u8>) {
        (*self as i64).encode(out);
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        isize::try_from(i64::decode(input)?).map_err(|_| DecodeError::InvalidData("isize trop grand"))
    }
}

impl Spill for bool {
    fn encode(&self, out: &mut Vec<u8>) {
        out.push(*self as u8);
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        match u8::decode(input)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::InvalidData("booléen invalide")),
        }
    }
}

impl Spill for char {
    fn encode(&self, out: &mut Vec<u8>) {
        (*self as u32).encode(out);
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        char::from_u32(u32::decode(input)?).ok_or(DecodeError::InvalidData("char invalide"))
    }
}

impl Spill for () {
    fn encode(&self, _out: &mut Vec<u8>) {}
    fn decode(_input: &mut &[u8]) -> Result<Self, DecodeError> {
        Ok(())
    }
}

impl Spill for String {
    fn encode(&self, out: &mut Vec<u8>) {
        self.len().encode(out);
        out.extend_from_slice(self.as_bytes());
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        let len = usize::decode(input)?;
        let bytes = take(input, len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::InvalidData("UTF-8 invalide"))
    }
    fn mem_size(&self) -> usize {
        size_of::<String>() + self.len()
    }
}

impl<T: Spill> Spill for Vec<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        self.len().encode(out);
        for x in self {
            x.encode(out);
        }
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        let len = usize::decode(input)?;
        // La capacité est bornée par les octets restants : une longueur corrompue
        // ne peut pas provoquer une allocation démesurée.
        let mut v = Vec::with_capacity(len.min(input.len()));
        for _ in 0..len {
            v.push(T::decode(input)?);
        }
        Ok(v)
    }
    fn mem_size(&self) -> usize {
        size_of::<Vec<T>>() + self.iter().map(Spill::mem_size).sum::<usize>()
    }
}

impl<T: Spill> Spill for Option<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            None => out.push(0),
            Some(x) => {
                out.push(1);
                x.encode(out);
            }
        }
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        match u8::decode(input)? {
            0 => Ok(None),
            1 => Ok(Some(T::decode(input)?)),
            _ => Err(DecodeError::InvalidData("tag Option invalide")),
        }
    }
    fn mem_size(&self) -> usize {
        match self {
            None => size_of::<Self>(),
            Some(x) => size_of::<Self>() + x.mem_size().saturating_sub(size_of::<T>()),
        }
    }
}

impl<T: Spill> Spill for Box<T> {
    fn encode(&self, out: &mut Vec<u8>) {
        (**self).encode(out);
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        Ok(Box::new(T::decode(input)?))
    }
    fn mem_size(&self) -> usize {
        size_of::<Box<T>>() + (**self).mem_size()
    }
}

impl<T: Spill, const N: usize> Spill for [T; N] {
    fn encode(&self, out: &mut Vec<u8>) {
        for x in self {
            x.encode(out);
        }
    }
    fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
        let mut v = Vec::with_capacity(N.min(input.len()));
        for _ in 0..N {
            v.push(T::decode(input)?);
        }
        v.try_into().map_err(|_| DecodeError::InvalidData("longueur de tableau"))
    }
    fn mem_size(&self) -> usize {
        self.iter().map(Spill::mem_size).sum()
    }
}

macro_rules! impl_tuple {
    ($($name:ident),+) => {
        impl<$($name: Spill),+> Spill for ($($name,)+) {
            #[allow(non_snake_case)]
            fn encode(&self, out: &mut Vec<u8>) {
                let ($($name,)+) = self;
                $($name.encode(out);)+
            }
            fn decode(input: &mut &[u8]) -> Result<Self, DecodeError> {
                Ok(($($name::decode(input)?,)+))
            }
            #[allow(non_snake_case)]
            fn mem_size(&self) -> usize {
                let ($($name,)+) = self;
                0 $(+ $name.mem_size())+
            }
        }
    };
}
impl_tuple!(A);
impl_tuple!(A, B);
impl_tuple!(A, B, C);
impl_tuple!(A, B, C, D);
impl_tuple!(A, B, C, D, E);
impl_tuple!(A, B, C, D, E, F);
impl_tuple!(A, B, C, D, E, F, G);
impl_tuple!(A, B, C, D, E, F, G, H);
