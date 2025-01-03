use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use serde_with::skip_serializing_none;
use utoipa::ToSchema;

/// Represents a tracker data revision value.
#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDataValue<TValue = serde_json::Value> {
    /// Original value retrieved during extraction phase.
    original: TValue,

    /// A list of values after applying modification tracker actions, if any.
    mods: Option<Vec<TValue>>,
}

impl<TValue> TrackerDataValue<TValue> {
    /// Create a new tracker data value based on the original value.
    pub fn new(original: TValue) -> Self {
        Self {
            original,
            mods: None,
        }
    }

    /// Adda a new modification for the tracker data value.
    pub fn add_mod(&mut self, mod_value: TValue) {
        self.mods.get_or_insert_with(Vec::new).push(mod_value);
    }

    /// Retrieve the final value after applying all modifications. If there are no modifications,
    /// the original value is returned.
    pub fn value(&self) -> &TValue {
        self.mods
            .as_ref()
            .and_then(|mods| mods.last())
            .unwrap_or(&self.original)
    }

    /// Returns the original data value.
    pub fn original(&self) -> &TValue {
        &self.original
    }

    /// Returns the list of modifications applied to the tracker data value, if any.
    pub fn mods(&self) -> Option<&Vec<TValue>> {
        self.mods.as_ref()
    }

    /// Consumes the tracker data value and returns the original value and the list of modifications.
    pub fn split(self) -> (TValue, Option<Vec<TValue>>) {
        (self.original, self.mods)
    }
}

impl TrackerDataValue<JsonValue> {
    /// Calculates the total size of the data value including the original and all modifications.
    /// Operation is expensive as it involves converting all values to strings.
    pub fn size(&self) -> usize {
        self.into_iter().map(|value| value.to_string().len()).sum()
    }
}

pub struct TrackerDataValueIter<'a, TValue> {
    data_value: &'a TrackerDataValue<TValue>,
    index: usize,
}

impl<'a, TValue> Iterator for TrackerDataValueIter<'a, TValue> {
    type Item = &'a TValue;
    fn next(&mut self) -> Option<Self::Item> {
        if self.index == 0 {
            self.index += 1;
            return Some(&self.data_value.original);
        }

        let mod_index = self.index - 1;
        match self.data_value.mods {
            Some(ref mod_value) if mod_index < mod_value.len() => {
                self.index += 1;
                Some(&mod_value[mod_index])
            }
            _ => None,
        }
    }
}

impl<'a, TValue> IntoIterator for &'a TrackerDataValue<TValue> {
    type Item = &'a TValue;
    type IntoIter = TrackerDataValueIter<'a, TValue>;

    fn into_iter(self) -> Self::IntoIter {
        TrackerDataValueIter {
            data_value: self,
            index: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::trackers::TrackerDataValue;
    use insta::assert_json_snapshot;
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let value = TrackerDataValue::new(json!("some-data"));
        assert_json_snapshot!(value, @r###"
        {
          "original": "some-data"
        }
        "###);
        assert_json_snapshot!(value.value(), @r###""some-data""###);

        let mut value = TrackerDataValue::new(json!("some-data"));
        value.add_mod(json!("some-mod"));
        value.add_mod(json!("another-mod"));
        assert_json_snapshot!(value, @r###"
        {
          "original": "some-data",
          "mods": [
            "some-mod",
            "another-mod"
          ]
        }
        "###);
        assert_json_snapshot!(value.value(), @r###""another-mod""###);

        Ok(())
    }

    #[test]
    fn can_iterate_through_all_values() -> anyhow::Result<()> {
        let value = TrackerDataValue::new(json!("some-data"));
        assert_json_snapshot!(value.into_iter().collect::<Vec<_>>(), @r###"
        [
          "some-data"
        ]
        "###);

        let mut value = TrackerDataValue::new(json!("some-data"));
        value.add_mod(json!("some-mod"));
        value.add_mod(json!("another-mod"));
        assert_json_snapshot!(value.into_iter().collect::<Vec<_>>(), @r###"
        [
          "some-data",
          "some-mod",
          "another-mod"
        ]
        "###);

        Ok(())
    }

    #[test]
    fn can_return_original_and_mods_data_values() -> anyhow::Result<()> {
        let value = TrackerDataValue::new(json!("some-data"));
        assert_json_snapshot!(value.original(), @r###""some-data""###);
        assert_json_snapshot!(value.mods(), @r###"null"###);

        let mut value = TrackerDataValue::new(json!("some-data"));
        value.add_mod(json!("some-mod"));
        value.add_mod(json!("another-mod"));
        assert_json_snapshot!(value.original(), @r###""some-data""###);
        assert_json_snapshot!(value.mods(), @r###"
        [
          "some-mod",
          "another-mod"
        ]
        "###);

        Ok(())
    }

    #[test]
    fn can_calculate_data_size() -> anyhow::Result<()> {
        let mut value = TrackerDataValue::new(json!("some-data"));
        assert_eq!(value.size(), 11);

        value.add_mod(json!("some-mod"));
        assert_eq!(value.size(), 21);

        value.add_mod(json!("another-mod"));
        assert_eq!(value.size(), 34);

        Ok(())
    }
}
