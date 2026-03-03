mod web_scraper_content_request;
mod web_scraper_debugging_info;
mod web_scraper_error_response;
mod web_scraper_success_response;

pub use self::{
    web_scraper_content_request::{
        WebScraperBackend, WebScraperContentRequest, WebScraperDebugOptions,
    },
    web_scraper_debugging_info::WebScraperDebugInfo,
    web_scraper_error_response::WebScraperErrorResponse,
    web_scraper_success_response::WebScraperSuccessResponse,
};
