#!/usr/bin/env bash
set -euo pipefail

OPTIONS_FILE="${TGGW_OPTIONS_FILE:-/data/options.json}"

opt_raw() {
  local key="$1"
  jq -r --arg k "$key" 'if (.[$k] != null) then .[$k] else empty end' "$OPTIONS_FILE"
}

set_str_opt() {
  local opt_key="$1"
  local env_var="$2"
  local cur=""
  cur="${!env_var-}"
  if [ -n "$cur" ]; then
    return 0
  fi
  local val
  val="$(opt_raw "$opt_key")"
  if [ -n "$val" ]; then
    export "$env_var=$val"
  fi
}

set_bool_opt() {
  local opt_key="$1"
  local env_var="$2"
  local cur=""
  cur="${!env_var-}"
  if [ -n "$cur" ]; then
    return 0
  fi
  local val
  val="$(jq -r --arg k "$opt_key" 'if (.[$k] | type) == "boolean" then (if .[$k] then "1" else "0" end) else empty end' "$OPTIONS_FILE")"
  if [ -n "$val" ]; then
    export "$env_var=$val"
  fi
}

set_int_opt() {
  local opt_key="$1"
  local env_var="$2"
  local cur=""
  cur="${!env_var-}"
  if [ -n "$cur" ]; then
    return 0
  fi
  local val
  val="$(jq -r --arg k "$opt_key" 'if (.[$k] | type) == "number" then (.[$k] | floor | tostring) else empty end' "$OPTIONS_FILE")"
  if [ -n "$val" ]; then
    export "$env_var=$val"
  fi
}

set_int_pos_opt() {
  local opt_key="$1"
  local env_var="$2"
  local cur=""
  cur="${!env_var-}"
  if [ -n "$cur" ]; then
    return 0
  fi
  local val
  val="$(jq -r --arg k "$opt_key" 'if (.[$k] | type) == "number" and .[$k] > 0 then (.[$k] | floor | tostring) else empty end' "$OPTIONS_FILE")"
  if [ -n "$val" ]; then
    export "$env_var=$val"
  fi
}

if [ -r "$OPTIONS_FILE" ]; then
  # Khi chạy dưới Home Assistant Supervisor, giữ /data/options.json nguyên quyền đọc
  # bằng cách tách thư mục trạng thái (UiAuth ép chmod 0700 lên TGGW_DATA).
  if [ "${TGGW_DATA:-/data}" = "/data" ] && [ -z "${DATA_DIR:-}" ]; then
    export TGGW_DATA="/data/state"
  fi
  mkdir -p "${TGGW_DATA:-/data}"
  chmod 0700 "${TGGW_DATA:-/data}" 2>/dev/null || true

  set_int_pos_opt "tg_api_id" "TG_API_ID"
  set_str_opt "tg_api_hash" "TG_API_HASH"
  set_str_opt "tg_phone" "TG_PHONE"
  set_str_opt "gateway_token" "GATEWAY_TOKEN"
  set_str_opt "tg_ui_user" "TG_UI_USER"
  set_str_opt "tg_ui_password" "TG_UI_PASSWORD"
  set_bool_opt "ui_cookie_secure" "TGGW_UI_COOKIE_SECURE"
  set_int_opt "rate_limit_per_minute" "RATE_LIMIT_PER_MINUTE"
  set_str_opt "ha_webhook_url" "HA_WEBHOOK_URL"
  set_str_opt "ha_webhook_token" "HA_WEBHOOK_TOKEN"
  set_bool_opt "telegram_p2p" "TELEGRAM_P2P"

  if [ -z "${VIDEO_PROFILE-}" ]; then
    vp="$(opt_raw "video_profile")"
    if [ "$vp" = "low" ]; then
      export VIDEO_PROFILE="low"
    fi
  fi

  set_str_opt "telegram_rx_video" "TELEGRAM_RX_VIDEO"
  set_str_opt "tts_voice" "TTS_VOICE"
  set_str_opt "tts_rate" "TTS_RATE"
  set_str_opt "log_level" "LOG_LEVEL"
  set_bool_opt "telegram_debug" "TELEGRAM_DEBUG"
  set_int_opt "calltrace_keep" "TELEGRAM_CALLTRACE_KEEP"
  set_int_opt "calltrace_keep_bad" "TELEGRAM_CALLTRACE_KEEP_BAD"
  set_int_opt "calltrace_max_kb" "TELEGRAM_CALLTRACE_MAX_KB"

  if [ -z "${TELEGRAM_WEBRTC_LOG-}" ]; then
    wlog="$(opt_raw "webrtc_log")"
    case "$wlog" in
      info|debug) export TELEGRAM_WEBRTC_LOG="$wlog" ;;
    esac
  fi

  set_int_opt "webrtc_log_mb" "TELEGRAM_WEBRTC_LOG_MB"
  set_str_opt "webrtc_log_loc" "TELEGRAM_WEBRTC_LOG_LOC"
fi

if [ "$#" -eq 0 ]; then
  exec /usr/local/bin/tggw serve
fi

exec /usr/local/bin/tggw "$@"
