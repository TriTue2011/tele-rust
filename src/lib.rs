//! Gateway Telegram bản Rust: hợp đồng giữa các module (`contract`) và các module hiện thực chúng.
//! `main.rs` chỉ là vỏ lệnh gọi lại thư viện này, để mỗi trait chỉ biên dịch MỘT lần.
pub mod calls;
pub mod chat;
pub mod contract;
pub mod rest;
pub mod td;
