use std::time::Duration;

use super::{
    page::CaptchaPage,
    types::{Detection, PageError},
};

pub async fn detect(_page: &impl CaptchaPage, _budget: Duration) -> Result<Detection, PageError> {
    Err(PageError::Browser("not implemented".into()))
}
