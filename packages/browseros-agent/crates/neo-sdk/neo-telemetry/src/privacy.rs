//! The span privacy dial (`SiteDetail`): what of an op's page URL a span may keep.
//!
//! Owned by the metrics-and-spans work. `none` keeps nothing, `host` keeps `url.domain`, `url`
//! adds `url.full` with userinfo, query string and fragment stripped. Spans only.
