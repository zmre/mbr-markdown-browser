//! The `Content-Security-Policy` header per route class
//! ([`mbr_core::csp::ContentClass`]).
//!
//! A handler that serves a repository file as it is marks its response
//! [`ContentClass::UserFiles`] ([`mark`]); every other response is classed by
//! its path ([`ContentClass::for_path`]). [`apply`] runs after the handler and
//! adds the class's header. With no policy configured the layer is not
//! installed at all, so responses are byte-identical to a server without it.

use axum::{
    Router,
    extract::Request,
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::Response,
};
use mbr_core::csp::{ContentClass, ContentSecurityPolicy};
use std::sync::Arc;

use crate::errors::ServerError;

/// The configured policies as header values, validated once at startup.
#[derive(Debug, Clone, Default)]
pub(crate) struct CspHeaders {
    pages: Option<HeaderValue>,
    user_files: Option<HeaderValue>,
    mbr_assets: Option<HeaderValue>,
}

impl CspHeaders {
    /// Validates every value as a header value. An empty (or blank) policy
    /// is no policy.
    pub(crate) fn new(policy: &ContentSecurityPolicy) -> Result<Self, ServerError> {
        let value = |class: &'static str, value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| {
                    HeaderValue::from_str(v).map_err(|_| ServerError::ContentSecurityPolicy {
                        class,
                        value: v.to_string(),
                    })
                })
                .transpose()
        };
        Ok(Self {
            pages: value("pages", policy.pages.as_deref())?,
            user_files: value("user_files", policy.user_files.as_deref())?,
            mbr_assets: value("mbr_assets", policy.mbr_assets.as_deref())?,
        })
    }

    fn is_empty(&self) -> bool {
        self.pages.is_none() && self.user_files.is_none() && self.mbr_assets.is_none()
    }

    fn for_class(&self, class: ContentClass) -> Option<&HeaderValue> {
        match class {
            ContentClass::Pages => self.pages.as_ref(),
            ContentClass::UserFiles => self.user_files.as_ref(),
            ContentClass::MbrAssets => self.mbr_assets.as_ref(),
        }
    }
}

/// Marks a response as being of `class`, overriding the path's class.
pub(crate) fn mark(mut response: Response, class: ContentClass) -> Response {
    response.extensions_mut().insert(class);
    response
}

async fn apply(headers: Arc<CspHeaders>, request: Request, next: Next) -> Response {
    let by_path = ContentClass::for_path(request.uri().path());
    let mut response = next.run(request).await;
    let class = response
        .extensions()
        .get::<ContentClass>()
        .copied()
        .unwrap_or(by_path);
    if let Some(value) = headers.for_class(class) {
        response
            .headers_mut()
            .insert(header::CONTENT_SECURITY_POLICY, value.clone());
    }
    response
}

/// Installs the header layer on `router` when any class has a policy.
pub(crate) fn layer<S>(router: Router<S>, headers: CspHeaders) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    if headers.is_empty() {
        return router;
    }
    let headers = Arc::new(headers);
    router.layer(middleware::from_fn(move |request: Request, next: Next| {
        apply(Arc::clone(&headers), request, next)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_policies_are_none_and_bad_ones_refused() {
        let headers = CspHeaders::new(&ContentSecurityPolicy {
            pages: Some("  ".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(headers.is_empty());

        let err = CspHeaders::new(&ContentSecurityPolicy {
            user_files: Some("sandbox\nallow-scripts".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(err.to_string().contains("user_files"), "{err}");
    }
}
