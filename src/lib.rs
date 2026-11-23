//! Gateway Telegram bản Rust: hợp đồng giữa các module (`contract`) và các module hiện thực chúng.
//! `main.rs` chỉ là vỏ lệnh gọi lại thư viện này, để mỗi trait chỉ biên dịch MỘT lần.
mod accounts;
mod account_files;
mod account_router;
mod appearance_settings;
mod login_http;
mod multi_feed;
mod storage;
mod storage_settings;
mod web;
pub mod app;
pub mod calls;
pub mod chat;
pub mod contract;
mod events;
pub mod rest;
mod tls_serve;
pub mod td;
pub mod ui_auth;
pub mod logging;
pub mod config;
