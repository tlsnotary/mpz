use std::collections::HashMap;

use mpz_circuits::types::ValueType;
use mpz_core::value::{ValueId, ValueRef};
use mpz_garble_core::{encoding_state::LabelState, EncodedValue};

use crate::MemoryError;

/// A registry of values.
///
/// This registry is used to track all the values that exist in a session.
///
/// It enforces that a value is only defined once, returning an error otherwise.
#[derive(Debug, Default)]
pub struct ValueRegistry {
    /// A map of value IDs to their types.
    values: HashMap<ValueId, ValueType>,
    /// A map of value IDs to their references.
    refs: HashMap<String, ValueRef>,
}

impl ValueRegistry {
    /// Adds a value to the registry.
    pub fn add_value(&mut self, id: &str, ty: ValueType) -> Result<ValueRef, MemoryError> {
        self.add_value_with_offset(id, ty, 0)
    }

    /// Adds a value to the registry, applying an offset to the ids of the elements if the
    /// value is an array.
    pub fn add_value_with_offset(
        &mut self,
        id: &str,
        ty: ValueType,
        offset: usize,
    ) -> Result<ValueRef, MemoryError> {
        let value_ref = match ty {
            ValueType::Array(elem_ty, len) => ValueRef::Array(
                (0..len)
                    .map(|idx| {
                        let id = ValueId::new(&format!("{}/{}", id, idx + offset));
                        self.add_value_id(id.clone(), (*elem_ty).clone())?;
                        Ok(id)
                    })
                    .collect::<Result<Vec<_>, MemoryError>>()?,
            ),
            _ => {
                let id = ValueId::new(id);
                self.add_value_id(id.clone(), ty)?;
                ValueRef::Value { id }
            }
        };

        self.refs.insert(id.to_string(), value_ref.clone());

        Ok(value_ref)
    }

    fn add_value_id(&mut self, id: ValueId, ty: ValueType) -> Result<(), MemoryError> {
        // Ensure that the value is not a collection.
        if matches!(ty, ValueType::Array(_, _)) {
            return Err(MemoryError::InvalidType(id, ty));
        }

        // Ensure that the value is not already defined.
        if self.values.contains_key(&id) {
            return Err(MemoryError::DuplicateValueId(id));
        }

        self.values.insert(id, ty);

        Ok(())
    }

    /// Returns a reference to the value with the given ID.
    pub(crate) fn get_value(&self, id: &str) -> Option<ValueRef> {
        self.refs.get(id).cloned()
    }

    /// Returns the type of the value with the given ID.
    pub(crate) fn get_value_type(&self, id: &str) -> Option<ValueType> {
        let value_ref = self.get_value(id)?;

        self.get_value_type_with_ref(&value_ref)
    }

    pub(crate) fn get_value_type_with_ref(&self, value: &ValueRef) -> Option<ValueType> {
        match value {
            ValueRef::Value { id } => self.values.get(id).cloned(),
            ValueRef::Array(values) => {
                let elem_tys = values
                    .iter()
                    .map(|id| self.values.get(id).cloned())
                    .collect::<Option<Vec<_>>>()?;

                // Ensure that all the elements have the same type.
                if elem_tys.windows(2).any(|window| window[0] != window[1]) {
                    return None;
                }

                Some(ValueType::Array(
                    Box::new(elem_tys[0].clone()),
                    values.len(),
                ))
            }
        }
    }
}

/// A unique ID for an encoding.
///
/// # Warning
///
/// The internal representation for this type is a `u64` and is computed using a hash function.
/// As such, it is not guaranteed to be unique and collisions may occur. Contexts using these
/// IDs should be aware of this and handle collisions appropriately.
///
/// For example, an encoding should never be used for more than one value as this will compromise
/// the security of the MPC protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub(crate) struct EncodingId(u64);

impl EncodingId {
    /// Create a new encoding ID.
    pub(crate) fn new(id: u64) -> Self {
        Self(id)
    }
}

impl From<u64> for EncodingId {
    fn from(id: u64) -> Self {
        Self::new(id)
    }
}

/// Errors which can occur when registering an encoding.
#[derive(Debug, thiserror::Error)]
pub enum EncodingRegistryError {
    #[error("encoding for value {0:?} is already defined")]
    DuplicateId(ValueId),
}

/// A registry of encodings.
///
/// This registry is used to store encodings for values.
///
/// It enforces that an encoding for a value is only set once.
#[derive(Debug)]
pub(crate) struct EncodingRegistry<T>
where
    T: LabelState,
{
    encodings: HashMap<EncodingId, EncodedValue<T>>,
}

impl<T> Default for EncodingRegistry<T>
where
    T: LabelState,
{
    fn default() -> Self {
        Self {
            encodings: HashMap::new(),
        }
    }
}

impl<T> EncodingRegistry<T>
where
    T: LabelState,
{
    /// Set the encoding for a value id.
    pub(crate) fn set_encoding_by_id(
        &mut self,
        id: &ValueId,
        encoding: EncodedValue<T>,
    ) -> Result<(), EncodingRegistryError> {
        let encoding_id = EncodingId::new(id.to_u64());
        if self.encodings.contains_key(&encoding_id) {
            return Err(EncodingRegistryError::DuplicateId(id.clone()));
        }

        self.encodings.insert(encoding_id, encoding);

        Ok(())
    }

    /// Set the encoding for a value.
    ///
    /// # Panics
    ///
    /// Panics if the encoding for the value has already been set, or if the value
    /// type does not match the encoding type.
    pub(crate) fn set_encoding(
        &mut self,
        value: &ValueRef,
        encoding: EncodedValue<T>,
    ) -> Result<(), EncodingRegistryError> {
        match (value, encoding) {
            (ValueRef::Value { id }, encoding) => self.set_encoding_by_id(id, encoding)?,
            (ValueRef::Array(ids), EncodedValue::Array(encodings))
                if ids.len() == encodings.len() =>
            {
                for (id, encoding) in ids.iter().zip(encodings) {
                    self.set_encoding_by_id(id, encoding)?
                }
            }
            _ => panic!("value type {:?} does not match encoding type", value),
        }

        Ok(())
    }

    /// Get the encoding for a value id if it exists.
    pub(crate) fn get_encoding_by_id(&self, id: &ValueId) -> Option<EncodedValue<T>> {
        self.encodings.get(&id.to_u64().into()).cloned()
    }

    /// Get the encoding for a value if it exists.
    ///
    /// # Panics
    ///
    /// Panics if the value is an array and if the type of its elements are not consistent.
    pub(crate) fn get_encoding(&self, value: &ValueRef) -> Option<EncodedValue<T>> {
        match value {
            ValueRef::Value { id, .. } => self.encodings.get(&id.to_u64().into()).cloned(),
            ValueRef::Array(ids) => {
                let encodings = ids
                    .iter()
                    .map(|id| self.encodings.get(&id.to_u64().into()).cloned())
                    .collect::<Option<Vec<_>>>()?;

                assert!(
                    encodings
                        .windows(2)
                        .all(|window| window[0].value_type() == window[1].value_type()),
                    "inconsistent element types in array {:?}",
                    value
                );

                Some(EncodedValue::Array(encodings))
            }
        }
    }

    /// Returns whether an encoding is present for a value id.
    pub(crate) fn contains(&self, id: &ValueId) -> bool {
        self.encodings.contains_key(&id.to_u64().into())
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;

    use super::*;

    use mpz_circuits::types::StaticValueType;
    use mpz_garble_core::{encoding_state, ChaChaEncoder, Encoder};
    use rstest::*;

    #[fixture]
    fn encoder() -> ChaChaEncoder {
        ChaChaEncoder::new([0; 32])
    }

    fn generate_encoding(
        encoder: ChaChaEncoder,
        value: &ValueRef,
        ty: &ValueType,
    ) -> EncodedValue<encoding_state::Full> {
        match (value, ty) {
            (ValueRef::Value { id }, ty) => encoder.encode_by_type(id.to_u64(), ty),
            (ValueRef::Array(ids), ValueType::Array(elem_ty, _)) => EncodedValue::Array(
                ids.iter()
                    .map(|id| encoder.encode_by_type(id.to_u64(), elem_ty))
                    .collect(),
            ),
            _ => panic!(),
        }
    }

    #[rstest]
    #[case::bit(PhantomData::<bool>)]
    #[case::u8(PhantomData::<u8>)]
    #[case::u16(PhantomData::<u16>)]
    #[case::u64(PhantomData::<u64>)]
    #[case::u64(PhantomData::<u64>)]
    #[case::u128(PhantomData::<u128>)]
    #[case::bit_array(PhantomData::<[bool; 16]>)]
    #[case::u8_array(PhantomData::<[u8; 16]>)]
    #[case::u16_array(PhantomData::<[u16; 16]>)]
    #[case::u32_array(PhantomData::<[u32; 16]>)]
    #[case::u64_array(PhantomData::<[u64; 16]>)]
    #[case::u128_array(PhantomData::<[u128; 16]>)]
    fn test_value_registry_duplicate_fails<T>(#[case] _ty: PhantomData<T>)
    where
        T: StaticValueType + Default + std::fmt::Debug,
    {
        let mut value_registry = ValueRegistry::default();

        let _ = value_registry.add_value("test", T::value_type()).unwrap();

        let err = value_registry
            .add_value("test", T::value_type())
            .unwrap_err();

        assert!(matches!(err, MemoryError::DuplicateValueId(_)));
    }

    #[rstest]
    #[case::bit(PhantomData::<bool>)]
    #[case::u8(PhantomData::<u8>)]
    #[case::u16(PhantomData::<u16>)]
    #[case::u64(PhantomData::<u64>)]
    #[case::u64(PhantomData::<u64>)]
    #[case::u128(PhantomData::<u128>)]
    #[case::bit_array(PhantomData::<[bool; 16]>)]
    #[case::u8_array(PhantomData::<[u8; 16]>)]
    #[case::u16_array(PhantomData::<[u16; 16]>)]
    #[case::u32_array(PhantomData::<[u32; 16]>)]
    #[case::u64_array(PhantomData::<[u64; 16]>)]
    #[case::u128_array(PhantomData::<[u128; 16]>)]
    fn test_encoding_registry_set_duplicate_fails<T>(
        encoder: ChaChaEncoder,
        #[case] _ty: PhantomData<T>,
    ) where
        T: StaticValueType + Default + std::fmt::Debug,
    {
        let mut value_registry = ValueRegistry::default();
        let mut full_encoding_registry = EncodingRegistry::<encoding_state::Full>::default();
        let mut active_encoding_registry = EncodingRegistry::<encoding_state::Active>::default();

        let ty = T::value_type();
        let value = value_registry.add_value("test", ty.clone()).unwrap();

        let encoding = generate_encoding(encoder, &value, &ty);

        full_encoding_registry
            .set_encoding(&value, encoding.clone())
            .unwrap();

        let err = full_encoding_registry
            .set_encoding(&value, encoding.clone())
            .unwrap_err();

        assert!(matches!(err, EncodingRegistryError::DuplicateId(_)));

        let encoding = encoding.select(T::default()).unwrap();

        active_encoding_registry
            .set_encoding(&value, encoding.clone())
            .unwrap();

        let err = active_encoding_registry
            .set_encoding(&value, encoding)
            .unwrap_err();

        assert!(matches!(err, EncodingRegistryError::DuplicateId(_)));
    }
}
