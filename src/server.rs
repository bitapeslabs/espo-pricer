use crate::{
    store::PriceStore,
    types::{PricePoint, PriceResponse},
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc};
use tracing::info;

const JSONRPC_VERSION: &str = "2.0";
const METHOD_GET_BTC_PRICE_AT_HEIGHT: &str = "get_btc_price_at_height";

#[derive(Clone)]
struct AppState {
    store: Arc<PriceStore>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    method: Option<String>,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    id: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub async fn serve(addr: SocketAddr, store: Arc<PriceStore>) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", post(handle_rpc))
        .with_state(AppState { store });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "RPC server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_rpc(
    State(state): State<AppState>,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    let id = request.id.clone();
    let response = match request.method.as_deref() {
        Some(METHOD_GET_BTC_PRICE_AT_HEIGHT) => get_btc_price_at_height(&state, id, request.params),
        Some(_) => error_response(id, -32601, "method not found"),
        None => error_response(id, -32600, "missing method"),
    };
    (StatusCode::OK, Json(response)).into_response()
}

fn get_btc_price_at_height(state: &AppState, id: Value, params: Value) -> JsonRpcResponse {
    let height = match parse_height_param(params) {
        Ok(height) => height,
        Err(message) => return error_response(id, -32602, &message),
    };

    match state.store.get_price_point(height) {
        Ok(Some(point)) => success_response(id, json!(to_price_response(point))),
        Ok(None) => error_response(id, -32004, "height has not been indexed"),
        Err(err) => error_response(id, -32000, &format!("failed to read price: {err}")),
    }
}

fn parse_height_param(params: Value) -> Result<u64, String> {
    match params {
        Value::Object(map) => map
            .get("height")
            .and_then(Value::as_u64)
            .ok_or_else(|| "params.height must be an unsigned integer".to_string()),
        Value::Array(items) if items.len() == 1 => items[0]
            .as_u64()
            .ok_or_else(|| "params[0] must be an unsigned integer".to_string()),
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| "params must be an unsigned integer".to_string()),
        _ => Err("params must be an object like {\"height\": 850000}".to_string()),
    }
}

fn to_price_response(point: PricePoint) -> PriceResponse {
    PriceResponse {
        price_scaled: point.price_scaled,
        price_raw: point.price_raw,
    }
}

fn success_response(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION,
        result: Some(result),
        error: None,
        id,
    }
}

fn error_response(id: Value, code: i64, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
        }),
        id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_object_height_param() {
        assert_eq!(
            parse_height_param(json!({ "height": 840000 })).unwrap(),
            840000
        );
    }

    #[test]
    fn rejects_missing_height_param() {
        assert!(parse_height_param(json!({})).is_err());
    }
}
