use std::{fmt, io, marker::PhantomData};

use borsh::BorshDeserialize;
use serde::{
    Deserialize, Deserializer,
    de::{self, SeqAccess, Visitor},
};

use crate::AccountId;

/// Never reserve from an untrusted sequence size hint or read past the bound.
pub(super) fn sequence<'de, D, T, const MAXIMUM: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct BoundedSequence<T, const MAXIMUM: usize>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>, const MAXIMUM: usize> Visitor<'de> for BoundedSequence<T, MAXIMUM> {
        type Value = Vec<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "an array containing at most {MAXIMUM} items")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            if sequence.size_hint().is_some_and(|size| size > MAXIMUM) {
                return Err(de::Error::custom("array exceeds its size limit"));
            }
            let mut values = Vec::new();
            while values.len() < MAXIMUM {
                let Some(value) = sequence.next_element()? else {
                    return Ok(values);
                };
                values.push(value);
            }
            sequence.next_element::<ExcessElement>()?;
            Ok(values)
        }
    }

    deserializer.deserialize_seq(BoundedSequence::<T, MAXIMUM>(PhantomData))
}

struct ExcessElement;

impl<'de> Deserialize<'de> for ExcessElement {
    fn deserialize<D: Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(de::Error::custom("array exceeds its size limit"))
    }
}

pub(super) fn text<'de, D: Deserializer<'de>, const MAXIMUM: usize>(
    deserializer: D,
) -> Result<String, D::Error> {
    struct BoundedText<const MAXIMUM: usize>;

    impl<'de, const MAXIMUM: usize> Visitor<'de> for BoundedText<MAXIMUM> {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "a string containing at most {MAXIMUM} UTF-8 bytes"
            )
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
            if value.len() > MAXIMUM {
                return Err(E::custom("string exceeds its size limit"));
            }
            Ok(value.to_owned())
        }

        fn visit_string<E: de::Error>(self, value: String) -> Result<String, E> {
            if value.len() > MAXIMUM {
                return Err(E::custom("string exceeds its size limit"));
            }
            Ok(value)
        }
    }

    deserializer.deserialize_string(BoundedText::<MAXIMUM>)
}

pub(super) fn account_id<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<AccountId, D::Error> {
    text::<D, 64>(deserializer)?
        .parse()
        .map_err(de::Error::custom)
}

pub(super) fn read_sequence<R: io::Read, T: BorshDeserialize, const MAXIMUM: usize>(
    reader: &mut R,
) -> io::Result<Vec<T>> {
    let length = u32::deserialize_reader(reader)?;
    if u64::from(length) > MAXIMUM as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "array exceeds its size limit",
        ));
    }
    (0..length).map(|_| T::deserialize_reader(reader)).collect()
}

pub(super) fn read_text<R: io::Read, const MAXIMUM: usize>(reader: &mut R) -> io::Result<String> {
    String::from_utf8(read_sequence::<R, u8, MAXIMUM>(reader)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(super) fn read_account_id<R: io::Read>(reader: &mut R) -> io::Result<AccountId> {
    read_text::<R, 64>(reader)?
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(super) mod decimal {
    use serde::{Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(value)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let value = super::text::<D, 20>(deserializer)?;
        if value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return Err(de::Error::custom("expected a canonical decimal u64 string"));
        }
        value.parse().map_err(de::Error::custom)
    }
}
