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

    let point = match height {
        HeightParam::Exact(height) => state.store.get_price_point(height),
        HeightParam::Latest => state.store.latest_price_point(),
    };

    match point {
        Ok(Some(point)) => success_response(id, json!(to_price_response(point))),
        Ok(None) => {
            let message = match height {
                HeightParam::Exact(_) => "height has not been indexed",
                HeightParam::Latest => "no indexed prices are available yet",
            };
            error_response(id, -32004, message)
        }
        Err(err) => error_response(id, -32000, &format!("failed to read price: {err}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeightParam {
    Exact(u64),
    Latest,
}

fn parse_height_param(params: Value) -> Result<HeightParam, String> {
    match params {
        Value::Null => Ok(HeightParam::Latest),
        Value::Object(map) => match map.get("height") {
            Some(value) => parse_height_value(value, "params.height"),
            None => Ok(HeightParam::Latest),
        },
        Value::Array(items) if items.is_empty() => Ok(HeightParam::Latest),
        Value::Array(items) if items.len() == 1 => parse_height_value(&items[0], "params[0]"),
        Value::Number(_) | Value::String(_) => parse_height_value(&params, "params"),
        _ => Err(
            "params must be omitted, empty, \"latest\", or include an unsigned integer height"
                .to_string(),
        ),
    }
}

fn parse_height_value(value: &Value, label: &str) -> Result<HeightParam, String> {
    match value {
        Value::Null => Ok(HeightParam::Latest),
        Value::Number(number) => number
            .as_u64()
            .map(HeightParam::Exact)
            .ok_or_else(|| format!("{label} must be an unsigned integer")),
        Value::String(raw) if raw.eq_ignore_ascii_case("latest") => Ok(HeightParam::Latest),
        Value::String(raw) => raw
            .parse::<u64>()
            .map(HeightParam::Exact)
            .map_err(|_| format!("{label} must be an unsigned integer or \"latest\"")),
        _ => Err(format!("{label} must be an unsigned integer or \"latest\"")),
    }
}

fn to_price_response(point: PricePoint) -> PriceResponse {
    PriceResponse {
        height: point.height,
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
            HeightParam::Exact(840000)
        );
    }

    #[test]
    fn defaults_missing_height_param_to_latest() {
        assert_eq!(
            parse_height_param(Value::Null).unwrap(),
            HeightParam::Latest
        );
        assert_eq!(parse_height_param(json!({})).unwrap(), HeightParam::Latest);
        assert_eq!(parse_height_param(json!([])).unwrap(), HeightParam::Latest);
        assert_eq!(
            parse_height_param(json!({ "height": "latest" })).unwrap(),
            HeightParam::Latest
        );
    }
}
