use serde::{de::Visitor, Deserializer};

// Serde's struct visitor also accepts positional fields in a JSON array.
// Request bodies and params are objects, not positional arguments.
pub(crate) struct Object<D>(pub D);

impl<'de, D: Deserializer<'de>> Deserializer<'de> for Object<D> {
    type Error = D::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.0.deserialize_map(visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier
        ignored_any
    }
}
