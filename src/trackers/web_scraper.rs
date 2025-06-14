mod web_scraper_content_request;
mod web_scraper_error_response;

pub use self::{
    web_scraper_content_request::{WebScraperBackend, WebScraperContentRequest},
    web_scraper_error_response::WebScraperErrorResponse,
};
