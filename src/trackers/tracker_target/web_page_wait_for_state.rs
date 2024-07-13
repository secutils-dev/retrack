use serde_derive::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Indicates the state of the element to wait for in a web page.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, Hash, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum WebPageWaitForState {
    /// Wait for the element to be present in the DOM of the page.
    Attached,
    /// Wait for the element to NOT be present in the DOM of the page.
    Detached,
    /// Wait for element to have non-empty bounding box and no `visibility: hidden`. The element
    /// without any content or with `display: none` has an empty bounding box and is not considered
    /// visible.
    Visible,
    /// Wait for element to be either detached from DOM, or have an empty bounding box or
    /// `visibility: hidden`. This is opposite to the 'Visible' state.
    Hidden,
}

#[cfg(test)]
mod tests {
    use super::WebPageWaitForState;
    use insta::assert_json_snapshot;
    use serde_json::json;

    #[test]
    fn serialization() -> anyhow::Result<()> {
        let state = WebPageWaitForState::Attached;
        assert_eq!(postcard::to_stdvec(&state)?, vec![0]);
        assert_json_snapshot!(state, @r###""attached""###);

        let state = WebPageWaitForState::Detached;
        assert_eq!(postcard::to_stdvec(&state)?, vec![1]);
        assert_json_snapshot!(state, @r###""detached""###);

        let state = WebPageWaitForState::Visible;
        assert_eq!(postcard::to_stdvec(&state)?, vec![2]);
        assert_json_snapshot!(state, @r###""visible""###);

        let state = WebPageWaitForState::Hidden;
        assert_eq!(postcard::to_stdvec(&state)?, vec![3]);
        assert_json_snapshot!(state, @r###""hidden""###);

        Ok(())
    }

    #[test]
    fn deserialization() -> anyhow::Result<()> {
        let state = WebPageWaitForState::Attached;
        assert_eq!(postcard::from_bytes::<WebPageWaitForState>(&[0])?, state);
        assert_eq!(
            serde_json::from_str::<WebPageWaitForState>(&json!("attached").to_string())?,
            state
        );

        let state = WebPageWaitForState::Detached;
        assert_eq!(postcard::from_bytes::<WebPageWaitForState>(&[1])?, state);
        assert_eq!(
            serde_json::from_str::<WebPageWaitForState>(&json!("detached").to_string())?,
            state
        );

        let state = WebPageWaitForState::Visible;
        assert_eq!(postcard::from_bytes::<WebPageWaitForState>(&[2])?, state);
        assert_eq!(
            serde_json::from_str::<WebPageWaitForState>(&json!("visible").to_string())?,
            state
        );

        let state = WebPageWaitForState::Hidden;
        assert_eq!(postcard::from_bytes::<WebPageWaitForState>(&[3])?, state);
        assert_eq!(
            serde_json::from_str::<WebPageWaitForState>(&json!("hidden").to_string())?,
            state
        );

        Ok(())
    }
}
