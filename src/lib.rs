//! Gateway Telegram bản Rust: hợp đồng giữa các module (`contract`) và các module hiện thực chúng.
//! `main.rs` chỉ là vỏ lệnh gọi lại thư viện này, để mỗi trait chỉ biên dịch MỘT lần.
mod accounts;
mod account_files;
mod account_router;
mod login_http;
mod multi_feed;
mod web;
pub mod app;
pub mod calls;
pub mod chat;
pub mod contract;
mod events;
pub mod rest;
pub mod td;
