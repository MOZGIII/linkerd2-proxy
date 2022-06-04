use crate::{
    r#match::{MatchHeader, MatchPath, MatchQueryParam, MatchRequest},
    MatchHost,
};
use linkerd2_proxy_api::http_route as api;

#[derive(Debug, thiserror::Error)]
#[error("host match must contain a match")]
pub struct HostMatchError;

#[derive(Debug, thiserror::Error)]
pub enum RouteMatchError {
    #[error("invalid path match: {0}")]
    Path(#[from] PathMatchError),

    #[error("invalid header match: {0}")]
    Header(#[from] HeaderMatchError),

    #[error("invalid query param match: {0}")]
    QueryParam(#[from] QueryParamMatchError),
}

#[derive(Debug, thiserror::Error)]
pub enum PathMatchError {
    #[error("missing match")]
    MissingMatch,

    #[error("invalid regular expression: {0}")]
    InvalidRegex(#[from] regex::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum HeaderMatchError {
    #[error("{0}")]
    InvalidName(#[from] http::header::InvalidHeaderName),

    #[error("missing a header value match")]
    MissingValueMatch,

    #[error("{0}")]
    InvalidValue(#[from] http::header::InvalidHeaderValue),

    #[error("invalid regular expression: {0}")]
    InvalidRegex(#[from] regex::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum QueryParamMatchError {
    #[error("missing a query param name")]
    MissingName,

    #[error("missing a query param value")]
    MissingValue,

    #[error("invalid regular expression: {0}")]
    InvalidRegex(#[from] regex::Error),
}

impl TryFrom<api::HostMatch> for MatchHost {
    type Error = HostMatchError;

    fn try_from(hm: api::HostMatch) -> Result<Self, Self::Error> {
        match hm.r#match.ok_or(HostMatchError)? {
            api::host_match::Match::Exact(h) => Ok(MatchHost::Exact(h)),
            api::host_match::Match::Suffix(sfx) => Ok(MatchHost::Suffix(sfx.reverse_labels)),
        }
    }
}

impl TryFrom<api::RouteMatch> for MatchRequest {
    type Error = RouteMatchError;

    fn try_from(rm: api::RouteMatch) -> Result<Self, Self::Error> {
        let path = match rm.path {
            None => None,
            Some(pm) => Some(pm.try_into()?),
        };
        Ok(MatchRequest {
            path,
            ..MatchRequest::default()
        })
    }
}

impl TryFrom<api::PathMatch> for MatchPath {
    type Error = PathMatchError;

    fn try_from(rm: api::PathMatch) -> Result<Self, Self::Error> {
        // TODO parse paths to validate they're valid.
        match rm.kind.ok_or(PathMatchError::MissingMatch)? {
            api::path_match::Kind::Exact(p) => Ok(MatchPath::Exact(p)),
            api::path_match::Kind::Prefix(p) => Ok(MatchPath::Prefix(p)),
            api::path_match::Kind::Regex(re) => Ok(MatchPath::Regex(re.parse()?)),
        }
    }
}

impl TryFrom<api::HeaderMatch> for MatchHeader {
    type Error = HeaderMatchError;

    fn try_from(hm: api::HeaderMatch) -> Result<Self, Self::Error> {
        let name = http::header::HeaderName::from_bytes(hm.name.as_bytes())?;
        match hm.value.ok_or(HeaderMatchError::MissingValueMatch)? {
            api::header_match::Value::Exact(h) => Ok(MatchHeader::Exact(name, h.parse()?)),
            api::header_match::Value::Regex(re) => Ok(MatchHeader::Regex(name, re.parse()?)),
        }
    }
}

impl TryFrom<api::QueryParamMatch> for MatchQueryParam {
    type Error = QueryParamMatchError;

    fn try_from(qpm: api::QueryParamMatch) -> Result<Self, Self::Error> {
        if qpm.name.is_empty() {
            return Err(QueryParamMatchError::MissingName);
        }
        match qpm.value.ok_or(QueryParamMatchError::MissingValue)? {
            api::query_param_match::Value::Exact(v) => Ok(MatchQueryParam::Exact(qpm.name, v)),
            api::query_param_match::Value::Regex(re) => {
                Ok(MatchQueryParam::Regex(qpm.name, re.parse()?))
            }
        }
    }
}
