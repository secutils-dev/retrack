use bytes::{Buf, Bytes};
use mediatype::{
    MediaType,
    names::{CSV, TEXT},
};
use tracing::{debug, warn};

/// Parser of the CSV files. Returns JSON representation of the parsed data as a binary
/// data. The JSON structure is a list of rows, each row is a list of cells.
pub struct CsvParser;
impl CsvParser {
    /// Check if the given media type is supported by the parser.
    pub fn supports(media_type: &MediaType) -> bool {
        media_type.ty == TEXT && media_type.subty == CSV
    }

    /// Parse the CSV file content and return JSON representation of the parsed data.
    pub fn parse(content: &[u8]) -> anyhow::Result<Bytes> {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .has_headers(false)
            .from_reader(content.reader());

        let mut rows = vec![];
        for (index, record) in reader.records().enumerate() {
            let record = match record {
                Ok(record) => record,
                Err(err) => {
                    warn!("Failed to parse CSV record with index {index}: {err:?}");
                    continue;
                }
            };

            rows.push(
                record
                    .into_iter()
                    .map(|cell| cell.to_string())
                    .collect::<Vec<_>>(),
            );
        }

        debug!("Parsed CSV file with {} rows.", rows.len());

        Ok(Bytes::from(serde_json::to_vec(&rows)?))
    }
}

#[cfg(test)]
mod tests {
    use super::CsvParser;
    use crate::tests::load_fixture;
    use insta::assert_json_snapshot;
    use mediatype::MediaTypeBuf;

    #[test]
    fn supports() -> anyhow::Result<()> {
        assert!(CsvParser::supports(
            &MediaTypeBuf::from_string("text/csv".to_string())?.to_ref()
        ));
        assert!(CsvParser::supports(
            &MediaTypeBuf::from_string("text/csv; charset=utf-8".to_string())?.to_ref()
        ));
        assert!(!CsvParser::supports(
            &MediaTypeBuf::from_string("application/json".to_string())?.to_ref()
        ));

        Ok(())
    }

    #[test]
    fn parse() -> anyhow::Result<()> {
        let fixture = load_fixture("csv_fixture.csv")?;
        let parsed_data = CsvParser::parse(&fixture)?;

        assert_json_snapshot!(
            serde_json::from_slice::<serde_json::Value>(&parsed_data)?,
            @r###"
        [
          [
            "Header N1",
            "Header N2",
            ""
          ],
          [
            "Some string",
            "100500",
            ""
          ],
          [
            "500100",
            "Some string 2",
            "100"
          ],
          [
            "",
            "",
            "Another string"
          ]
        ]
        "###
        );

        Ok(())
    }
}
