use bytes::Bytes;
use calamine::Reader;
use mediatype::{
    names::{
        vnd::{MS_EXCEL, OPENXMLFORMATS_OFFICEDOCUMENT_SPREADSHEETML_SHEET},
        APPLICATION,
    },
    MediaType,
};
use serde::Serialize;
use std::io::{BufReader, Cursor};
use tracing::debug;

/// Parser of the XLS/XLSX/XLSB files. Returns JSON representation of the parsed data as a binary
/// data. The JSON structure is a list of sheets, each sheet is an object with `name` and `data`.
/// The `data` is a list of rows, each row is a list of cells.
pub struct XlsParser;
impl XlsParser {
    /// Check if the given media type is supported by the parser.
    pub fn supports(media_type: &MediaType) -> bool {
        media_type.ty == APPLICATION
            && (media_type.subty == MS_EXCEL
                || media_type.subty == OPENXMLFORMATS_OFFICEDOCUMENT_SPREADSHEETML_SHEET)
    }

    /// Parse the XLS/XLSX/XLSB file content and return JSON representation of the parsed data.
    pub fn parse(content: &[u8]) -> anyhow::Result<Bytes> {
        #[derive(Serialize, Debug, PartialEq, Eq)]
        #[serde(rename_all = "camelCase")]
        struct Sheet {
            name: String,
            data: Vec<Vec<String>>,
        }

        let worksheets = calamine::Xlsx::new(BufReader::new(Cursor::new(content)))
            .map(|mut workbook| workbook.worksheets())
            .or_else(|_| {
                calamine::Xls::new(BufReader::new(Cursor::new(content)))
                    .map(|mut workbook| workbook.worksheets())
            })?;

        let mut sheets = vec![];
        for (sheet_name, range) in worksheets {
            let mut sheet_rows = vec![];
            for row in range.rows() {
                let mut sheet_cells = vec![];
                for cell in row {
                    sheet_cells.push(cell.to_string());
                }
                sheet_rows.push(sheet_cells);
            }

            debug!(
                "Parsed XLSX sheet '{sheet_name}' with {} rows.",
                sheet_rows.len()
            );

            sheets.push(Sheet {
                name: sheet_name,
                data: sheet_rows,
            });
        }

        Ok(Bytes::from(serde_json::to_vec(&sheets)?))
    }
}

#[cfg(test)]
mod tests {
    use super::XlsParser;
    use crate::tests::load_fixture;
    use insta::assert_json_snapshot;
    use mediatype::MediaTypeBuf;

    #[test]
    fn supports() -> anyhow::Result<()> {
        assert!(XlsParser::supports(
            &MediaTypeBuf::from_string("application/vnd.ms-excel".to_string())?.to_ref()
        ));
        assert!(XlsParser::supports(
            &MediaTypeBuf::from_string(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()
            )?
            .to_ref()
        ));
        assert!(!XlsParser::supports(
            &MediaTypeBuf::from_string("application/json".to_string())?.to_ref()
        ));

        Ok(())
    }

    #[test]
    fn parse_xlsx() -> anyhow::Result<()> {
        let fixture = load_fixture("xlsx_fixture.xlsx")?;
        let parsed_data = XlsParser::parse(&fixture)?;

        assert_json_snapshot!(
            serde_json::from_slice::<serde_json::Value>(&parsed_data)?,
            @r###"
        [
          {
            "name": "Sheet N1",
            "data": [
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
          },
          {
            "name": "Sheet N2",
            "data": [
              [
                "Header N3",
                "Header N4",
                ""
              ],
              [
                "Some string 3",
                "100500",
                ""
              ],
              [
                "600200",
                "Some string 4",
                "200"
              ],
              [
                "",
                "",
                "Another string 2"
              ]
            ]
          }
        ]
        "###
        );

        Ok(())
    }

    #[test]
    fn parse_xls() -> anyhow::Result<()> {
        let fixture = load_fixture("xls_fixture.xls")?;
        let parsed_data = XlsParser::parse(&fixture)?;

        assert_json_snapshot!(
            serde_json::from_slice::<serde_json::Value>(&parsed_data)?,
            @r###"
        [
          {
            "name": "Sheet N1",
            "data": [
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
          },
          {
            "name": "Sheet N2",
            "data": [
              [
                "Header N3",
                "Header N4",
                ""
              ],
              [
                "Some string 3",
                "100500",
                ""
              ],
              [
                "600200",
                "Some string 4",
                "200"
              ],
              [
                "",
                "",
                "Another string 2"
              ]
            ]
          }
        ]
        "###
        );

        Ok(())
    }
}
