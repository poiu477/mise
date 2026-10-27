//! Path and query extractors that reject bad input with the API's JSON errors (axum's own answer in plain
//! text), and refuse NUL characters, which Postgres cannot store or compare.

use axum::{extract::FromRequestParts, http::request::Parts};
use serde::de::DeserializeOwned;

use crate::error::ApiError;

fn reject_nul(parts: &Parts) -> Result<(), ApiError> {
    let uri = parts.uri.path_and_query().map_or("", |p| p.as_str());
    if uri.contains("%00") || uri.contains('\0') {
        return Err(ApiError::BadRequest("input may not contain NUL characters".into()));
    }
    Ok(())
}

pub struct Query<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for Query<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        reject_nul(parts)?;
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|q| Query(q.0))
            .map_err(|e| ApiError::BadRequest(e.body_text()))
    }
}

pub struct Path<T>(pub T);

impl<T: DeserializeOwned + Send, S: Send + Sync> FromRequestParts<S> for Path<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        reject_nul(parts)?;
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|p| Path(p.0))
            .map_err(|e| ApiError::BadRequest(e.body_text()))
    }
}
