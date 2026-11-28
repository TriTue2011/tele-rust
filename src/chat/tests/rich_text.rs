use super::*;
use crate::chat::rich_text::{
    apply_auto_emphasis, from_markdown, is_allowed_url, parse_options, preserves_original_text,
    render, render_tdlib, utf16_len, EntityKind, FormatMode, RichEntity, RichOptions,
    MAX_ENTITIES, MAX_RAW_UTF16, MAX_TEXT_UTF16,
};
use axum::{
    body::{to_bytes, Body},
    http::{header, Request},
};
use tower::ServiceExt;

use crate::rest::{self, RestConfig};
use crate::ui_auth::UiAuth;

fn ent(offset: usize, length: usize, kind: EntityKind) -> RichEntity {
    RichEntity { offset, length, kind }
}

fn plain_opts(auto_emphasis: bool) -> RichOptions<'static> {
    RichOptions { format: FormatMode::Plain, auto_emphasis, ai_markup: None }
}

fn md_opts(auto_emphasis: bool) -> RichOptions<'static> {
    RichOptions { format: FormatMode::Markdown, auto_emphasis, ai_markup: None }
}

fn ai_opts(ai_markup: &str, auto_emphasis: bool) -> RichOptions<'_> {
    RichOptions { format: FormatMode::Plain, auto_emphasis, ai_markup: Some(ai_markup) }
}

#[test]
fn markdown_table_maps_all_formats_nesting_utf16_emoji_nfd_escape_unclosed_and_urls() {
    // 1. Bảng markdown chuẩn Telegram (9 loại entity).
    let simple_cases = [
        ("**đậm**", "đậm", vec![ent(0, 3, EntityKind::Bold)]),
        ("*nghiêng*", "nghiêng", vec![ent(0, 7, EntityKind::Italic)]),
        ("_nghiêng_", "nghiêng", vec![ent(0, 7, EntityKind::Italic)]),
        ("__gạch chân__", "gạch chân", vec![ent(0, 9, EntityKind::Underline)]),
        ("~~gạch giữa~~", "gạch giữa", vec![ent(0, 9, EntityKind::Strikethrough)]),
        ("`mã inline`", "mã inline", vec![ent(0, 9, EntityKind::Code)]),
        ("`**không đậm**`", "**không đậm**", vec![ent(0, 13, EntityKind::Code)]),
        ("``` khối mã ```", "khối mã", vec![ent(0, 7, EntityKind::Pre(None))]),
        (
            "```rust\nlet x = 1;\n```",
            "let x = 1;",
            vec![ent(0, 10, EntityKind::Pre(Some("rust".into())))],
        ),
        ("||bí mật||", "bí mật", vec![ent(0, 6, EntityKind::Spoiler)]),
        ("> dòng trích dẫn", "dòng trích dẫn", vec![ent(0, 14, EntityKind::BlockQuote)]),
        (
            "> dòng 1\n> dòng 2",
            "dòng 1\ndòng 2",
            vec![ent(0, 13, EntityKind::BlockQuote)],
        ),
        (
            "> **đậm** trong trích dẫn",
            "đậm trong trích dẫn",
            vec![
                ent(0, 19, EntityKind::BlockQuote),
                ent(0, 3, EntityKind::Bold),
            ],
        ),
        (
            "[Trang chủ](https://example.com/a)",
            "Trang chủ",
            vec![ent(0, 9, EntityKind::TextUrl("https://example.com/a".into()))],
        ),
        (
            "[HTTP](http://example.org/p?x=1)",
            "HTTP",
            vec![ent(0, 4, EntityKind::TextUrl("http://example.org/p?x=1".into()))],
        ),
        (
            "[Mở Tele](tg://resolve?domain=demo)",
            "Mở Tele",
            vec![ent(0, 7, EntityKind::TextUrl("tg://resolve?domain=demo".into()))],
        ),
        (
            "[**Liên kết đậm**](https://example.com)",
            "Liên kết đậm",
            vec![
                ent(0, 12, EntityKind::TextUrl("https://example.com".into())),
                ent(0, 12, EntityKind::Bold),
            ],
        ),
        (
            "[Rust](https://example.org/wiki/Rust_(lang))",
            "Rust",
            vec![ent(0, 4, EntityKind::TextUrl("https://example.org/wiki/Rust_(lang)".into()))],
        ),
    ];
    for (src, want_text, want_entities) in simple_cases {
        let got = from_markdown(src);
        assert_eq!(got.text, want_text, "src={src}");
        assert_eq!(got.entities, want_entities, "src={src}");
    }

    // 2. Lồng nhau & chồng lấn hợp lệ.
    let nested_cases = [
        (
            "***đậm nghiêng***",
            "đậm nghiêng",
            vec![ent(0, 11, EntityKind::Bold), ent(0, 11, EntityKind::Italic)],
        ),
        (
            "**đậm *nghiêng***",
            "đậm nghiêng",
            vec![ent(0, 11, EntityKind::Bold), ent(4, 7, EntityKind::Italic)],
        ),
        (
            "*nghiêng **đậm***",
            "nghiêng đậm",
            vec![ent(0, 11, EntityKind::Italic), ent(8, 3, EntityKind::Bold)],
        ),
        (
            "**đậm _cả hai** nghiêng_",
            "đậm cả hai nghiêng",
            vec![ent(0, 10, EntityKind::Bold), ent(4, 14, EntityKind::Italic)],
        ),
        (
            "**a [b](https://example.com) c**",
            "a b c",
            vec![
                ent(0, 5, EntityKind::Bold),
                ent(2, 1, EntityKind::TextUrl("https://example.com".into())),
            ],
        ),
        (
            "Tiêu đề\n> **đậm**",
            "Tiêu đề\nđậm",
            vec![ent(8, 3, EntityKind::BlockQuote), ent(8, 3, EntityKind::Bold)],
        ),
    ];
    for (src, want_text, want_entities) in nested_cases {
        let got = from_markdown(src);
        assert_eq!(got.text, want_text, "src={src}");
        assert_eq!(got.entities, want_entities, "src={src}");
    }

    // 3. Giữ nguyên '_' trong từ và '*' đứng riêng.
    for raw in ["sensor.nhiet_do_phong_khach", "2 * 3 * 4"] {
        let got = from_markdown(raw);
        assert_eq!(got.text, raw);
        assert!(got.entities.is_empty(), "raw={raw}");
    }

    // 4. Emoji surrogate pair (😀 = 2 UTF-16) và chữ Việt tổ hợp NFD.
    let emoji = from_markdown("😀 **đậm**");
    assert_eq!(emoji.text, "😀 đậm");
    assert_eq!(emoji.entities, vec![ent(3, 3, EntityKind::Bold)]);

    let nfd = from_markdown("Chao\u{0300} **ba\u{0323}n**");
    assert_eq!(nfd.text, "Chao\u{0300} ba\u{0323}n");
    assert_eq!(nfd.entities, vec![ent(6, 4, EntityKind::Bold)]);

    // 5. Escape bằng '\'.
    assert_eq!(from_markdown(r"\*\*không đậm\*\*").text, "**không đậm**");
    assert!(from_markdown(r"\*\*không đậm\*\*").entities.is_empty());
    assert_eq!(from_markdown(r"\_không nghiêng\_").text, "_không nghiêng_");
    assert!(from_markdown(r"\_không nghiêng\_").entities.is_empty());
    assert_eq!(from_markdown(r"\> không trích dẫn").text, "> không trích dẫn");
    assert!(from_markdown(r"\> không trích dẫn").entities.is_empty());
    let esc_bs = from_markdown(r"\\**đậm**");
    assert_eq!(esc_bs.text, r"\đậm");
    assert_eq!(esc_bs.entities, vec![ent(1, 3, EntityKind::Bold)]);

    // 6. Markdown không đóng ⇒ giữ nguyên chữ (0 mất chữ).
    for unclosed in [
        "**chưa đóng",
        "*chưa đóng",
        "__chưa đóng",
        "~~chưa đóng",
        "||chưa đóng",
        "`chưa đóng",
        "```chưa đóng",
        "[chưa đóng](https://example.com",
        "> ",
        "****",
        "**~~**~~",
    ] {
        let got = from_markdown(unclosed);
        assert_eq!(got.text, unclosed, "unclosed={unclosed}");
        assert!(got.entities.is_empty(), "unclosed={unclosed}");
    }

    // 7. Liên kết lạ bị từ chối (0 TextUrl, không mất chữ).
    for bad_link in [
        "[bấm](javascript:alert(1))",
        "[bấm](JavaScript:alert(1))",
        "[bấm](data:text/html,<script>alert(1)</script>)",
        "[bấm](file:///etc/passwd)",
        "[bấm](ftp://example.com/a)",
        "[bấm](vbscript:msgbox(1))",
        "[bấm](https://)",
    ] {
        let got = from_markdown(bad_link);
        assert_eq!(got.text, bad_link, "bad_link={bad_link}");
        assert!(
            !got.entities.iter().any(|e| matches!(e.kind, EntityKind::TextUrl(_))),
            "bad_link={bad_link}"
        );
    }

    // 8. Mã inline chứa dấu giống block không bị vỡ.
    let inline_block = from_markdown("`> not quote` và `a ``` b`");
    assert_eq!(inline_block.text, "> not quote và a ``` b");
    assert_eq!(
        inline_block.entities,
        vec![
            ent(0, 11, EntityKind::Code),
            ent(15, 7, EntityKind::Code),
        ]
    );
}

#[test]
fn auto_emphasis_bolds_numbers_with_units_and_skips_formatted_code_and_urls() {
    let basic = render("Nhiệt độ 29°C, độ ẩm 75%, còn 3 giờ 15 phút.", &plain_opts(true)).unwrap();
    assert_eq!(basic.text, "Nhiệt độ 29°C, độ ẩm 75%, còn 3 giờ 15 phút.");
    assert_eq!(
        basic.entities,
        vec![
            ent(9, 4, EntityKind::Bold),
            ent(21, 3, EntityKind::Bold),
            ent(30, 5, EntityKind::Bold),
            ent(36, 7, EntityKind::Bold),
        ]
    );

    let full = render(
        "😀 Trời -5°C, mưa 12,5 mm, gió 30 km/h, 29 độ C, nặng 2.5 kg, dài 100 m, 45 giây, 7 ngày.",
        &plain_opts(true),
    )
    .unwrap();
    assert_eq!(
        full.entities,
        vec![
            ent(8, 4, EntityKind::Bold),
            ent(18, 7, EntityKind::Bold),
            ent(31, 7, EntityKind::Bold),
            ent(40, 7, EntityKind::Bold),
            ent(54, 6, EntityKind::Bold),
            ent(66, 5, EntityKind::Bold),
            ent(73, 7, EntityKind::Bold),
            ent(82, 6, EntityKind::Bold),
        ]
    );

    // Chữ Việt tổ hợp NFD với emoji phía trước.
    let nfd = render("😀 Còn 3 gio\u{031B}\u{0300}", &plain_opts(true)).unwrap();
    assert_eq!(nfd.text, "😀 Còn 3 gio\u{031B}\u{0300}");
    assert_eq!(nfd.entities, vec![ent(7, 7, EntityKind::Bold)]);

    // Không đụng vùng đã có mã, định dạng markdown hoặc URL trần.
    let mixed = render(
        "Lệnh `sleep 30s` và ```\n29°C\n``` và **75%** và https://example.com/50km còn 10 phút",
        &md_opts(true),
    )
    .unwrap();
    assert_eq!(
        mixed.text,
        "Lệnh sleep 30s và 29°C và 75% và https://example.com/50km còn 10 phút"
    );
    assert_eq!(
        mixed.entities,
        vec![
            ent(5, 9, EntityKind::Code),
            ent(18, 4, EntityKind::Pre(None)),
            ent(26, 3, EntityKind::Bold),
            ent(62, 7, EntityKind::Bold),
        ]
    );

    // Không ăn '-' trong dãy 10-20°C và không ăn định danh model-3g.
    let range_text = render("Khoảng 10-20°C và model-3g", &plain_opts(true)).unwrap();
    assert_eq!(range_text.entities, vec![ent(10, 4, EntityKind::Bold)]);

    // Không nhấn mạnh nhầm từ không phải đơn vị đo.
    let non_units = render("Cảnh báo khẩn cấp: có 3 máy và 2 người ở phòng 4", &plain_opts(true)).unwrap();
    assert!(non_units.entities.is_empty());
}

#[tokio::test]
async fn ai_markup_accepts_preserved_text_and_rejects_modified_text_with_422() {
    // 1. Hợp lệ: chèn dấu markdown + emoji trang trí quanh chữ gốc (kết hợp auto_emphasis).
    let raw_text = "Phòng khách 29°C, độ ẩm 75%";
    let markup = "🌡️ **Phòng khách** 29°C, độ ẩm 75% ✅";
    let expected_plain = "🌡️ Phòng khách 29°C, độ ẩm 75% ✅";
    let expected_ft = json!({
        "@type": "formattedText",
        "text": expected_plain,
        "entities": [
            {"@type":"textEntity","offset":4,"length":11,"type":{"@type":"textEntityTypeBold"}},
            {"@type":"textEntity","offset":16,"length":4,"type":{"@type":"textEntityTypeBold"}},
            {"@type":"textEntity","offset":28,"length":3,"type":{"@type":"textEntityTypeBold"}}
        ]
    });
    let expected_legacy_ft = json!({
        "@type": "formattedText",
        "text": "Nhiệt độ 29°C",
        "entities": [
            {"@type":"textEntity","offset":9,"length":4,"type":{"@type":"textEntityTypeBold"}}
        ]
    });

    let (chat, _) = setup(vec![
        ok(
            json!({
                "@type": "sendMessage",
                "chat_id": 5,
                "input_message_content": {
                    "@type": "inputMessageText",
                    "text": expected_ft,
                    "link_preview_options": {"@type":"linkPreviewOptions","is_disabled":false}
                }
            }),
            td_message(50),
        ),
        ok(json!({"@type":"createPrivateChat","user_id":7,"force":false}), json!({"id":5})),
        ok(
            json!({
                "@type": "sendMessage",
                "chat_id": 5,
                "input_message_content": {
                    "@type": "inputMessageText",
                    "text": expected_legacy_ft,
                    "link_preview_options": {"@type":"linkPreviewOptions","is_disabled":true}
                }
            }),
            td_message(51),
        ),
    ]);

    let sent = post(
        &chat,
        "/api/chats/5/send",
        "",
        json!({"text": raw_text, "ai_markup": markup, "auto_emphasis": true}),
    )
    .await
    .unwrap();
    assert_eq!(sent["id"], 50);

    let legacy_sent = post(
        &chat,
        "/api/send_message",
        "",
        json!({"peer": "7", "message": "Nhiệt độ 29°C", "ai_markup": "Nhiệt độ **29°C**"}),
    )
    .await
    .unwrap();
    assert_eq!(legacy_sent["tin_id"], "51");

    // 2. Bị sửa chữ gốc ⇒ 422 AI_MARKUP_MISMATCH và không gọi TDLib.
    let (strict_chat, _) = setup(vec![]);
    for (path, body) in [
        (
            "/api/chats/5/send",
            json!({"text":"Nhiệt độ 29°C","ai_markup":"Nhiệt độ **30°C**"}),
        ),
        (
            "/api/chats/5/send",
            json!({"text":"Cửa mở!","ai_markup":"**Cửa mở**"}),
        ),
        (
            "/api/chats/5/send",
            json!({"text":"Cửa mở","ai_markup":"Cửa phòng mở"}),
        ),
        (
            "/api/chats/5/send",
            json!({"text":"Cảnh báo 🔥","ai_markup":"Cảnh báo ❄️"}),
        ),
        (
            "/api/chats/5/send",
            json!({"text":"Nhiệt độ 29°C","ai_markup":"[Nhiệt độ 29°C](javascript:alert(1))"}),
        ),
        (
            "/api/chats/5/send",
            json!({"text":"Nhiệt độ 29°C","ai_markup":""}),
        ),
        (
            "/api/send_message",
            json!({"peer":"7","message":"Nhiệt độ 29°C","ai_markup":"Nhiệt độ 30°C"}),
        ),
    ] {
        let err = post(&strict_chat, path, "", body.clone()).await.unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (422, "AI_MARKUP_MISMATCH"), "{body}");
    }
}

#[tokio::test]
async fn backward_compatibility_and_all_text_routes_with_fake_tdlib() {
    let raw_legacy = "**giữ nguyên** 29°C";
    let ft_plain = json!({"@type":"formattedText","text":raw_legacy,"entities":[]});
    let ft_md_auto = json!({
        "@type":"formattedText",
        "text":"Báo cáo 29°C",
        "entities":[
            {"@type":"textEntity","offset":0,"length":7,"type":{"@type":"textEntityTypeBold"}},
            {"@type":"textEntity","offset":8,"length":4,"type":{"@type":"textEntityTypeBold"}}
        ]
    });
    let ft_edit = json!({
        "@type":"formattedText",
        "text":"Sửa 75%",
        "entities":[
            {"@type":"textEntity","offset":0,"length":3,"type":{"@type":"textEntityTypeItalic"}},
            {"@type":"textEntity","offset":4,"length":3,"type":{"@type":"textEntityTypeBold"}}
        ]
    });
    let ft_cap = json!({
        "@type":"formattedText",
        "text":"Ảnh 29°C",
        "entities":[
            {"@type":"textEntity","offset":0,"length":3,"type":{"@type":"textEntityTypeBold"}},
            {"@type":"textEntity","offset":4,"length":4,"type":{"@type":"textEntityTypeBold"}}
        ]
    });
    let ft_album_2 = json!({
        "@type":"formattedText",
        "text":"Đoạn 3 giờ",
        "entities":[{"@type":"textEntity","offset":5,"length":5,"type":{"@type":"textEntityTypeBold"}}]
    });

    let file = |name: &str, bytes: &str| {
        json!({"@type":"inputFileLocal","path":name,"bytes":bytes})
    };
    let (chat, _) = setup(vec![
        // 1. Không truyền tham số mới trên /send -> entities rỗng như cũ.
        ok(
            json!({
                "@type":"sendMessage","chat_id":5,
                "input_message_content":{"@type":"inputMessageText","text":ft_plain.clone(),
                    "link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}}
            }),
            td_message(60),
        ),
        // 2. Không truyền tham số mới trên /api/send_message -> entities rỗng như cũ.
        ok(json!({"@type":"createPrivateChat","user_id":7,"force":false}), json!({"id":5})),
        ok(
            json!({
                "@type":"sendMessage","chat_id":5,
                "input_message_content":{"@type":"inputMessageText","text":ft_plain,
                    "link_preview_options":{"@type":"linkPreviewOptions","is_disabled":true}}
            }),
            td_message(61),
        ),
        // 3. /send với format=markdown + auto_emphasis=true.
        ok(
            json!({
                "@type":"sendMessage","chat_id":5,
                "input_message_content":{"@type":"inputMessageText","text":ft_md_auto.clone(),
                    "link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}}
            }),
            td_message(62),
        ),
        // 4. /api/send_message với format=markdown + auto_emphasis=true.
        ok(json!({"@type":"createPrivateChat","user_id":7,"force":false}), json!({"id":5})),
        ok(
            json!({
                "@type":"sendMessage","chat_id":5,
                "input_message_content":{"@type":"inputMessageText","text":ft_md_auto,
                    "link_preview_options":{"@type":"linkPreviewOptions","is_disabled":true}}
            }),
            td_message(63),
        ),
        // 5. /api/messages/edit.
        ok(
            json!({
                "@type":"editMessageText","chat_id":5,"message_id":62,
                "input_message_content":{"@type":"inputMessageText","text":ft_edit}
            }),
            td_message(62),
        ),
        // 6. /api/send_media (caption).
        ok(json!({"@type":"createPrivateChat","user_id":7,"force":false}), json!({"id":5})),
        ok(
            json!({
                "@type":"sendMessage","chat_id":5,
                "input_message_content":{"@type":"inputMessageDocument","caption":ft_cap.clone(),
                    "document":{"@type":"inputDocument","document":file("note.txt","ok")}}
            }),
            td_message(64),
        ),
        // 7. /api/chats/5/send_file (query format=markdown&auto_emphasis=true).
        ok(
            json!({
                "@type":"sendMessage","chat_id":5,
                "input_message_content":{"@type":"inputMessagePhoto","caption":ft_cap.clone(),
                    "photo":{"@type":"inputPhoto","photo":file("a.jpg","jpg"),"width":0,"height":0,"added_sticker_file_ids":[]}}
            }),
            td_message(65),
        ),
        // 8. /api/chats/5/send_album.
        ok(
            json!({
                "@type":"sendMessageAlbum","chat_id":5,
                "input_message_contents":[
                    {"@type":"inputMessagePhoto","caption":ft_cap,
                        "photo":{"@type":"inputPhoto","photo":file("p1.jpg","p1"),"width":0,"height":0,"added_sticker_file_ids":[]}},
                    {"@type":"inputMessagePhoto","caption":ft_album_2,
                        "photo":{"@type":"inputPhoto","photo":file("p2.jpg","p2"),"width":0,"height":0,"added_sticker_file_ids":[]}}
                ]
            }),
            json!({"messages":[td_message(66),td_message(67)]}),
        ),
    ]);

    assert_eq!(post(&chat, "/api/chats/5/send", "", json!({"text": raw_legacy})).await.unwrap()["id"], 60);
    assert_eq!(post(&chat, "/api/send_message", "", json!({"peer":"7","message":raw_legacy})).await.unwrap()["tin_id"], "61");
    assert_eq!(
        post(&chat, "/api/chats/5/send", "", json!({"text":"**Báo cáo** 29°C","format":"markdown","auto_emphasis":true})).await.unwrap()["id"],
        62
    );
    assert_eq!(
        post(&chat, "/api/send_message", "", json!({"peer":"7","message":"**Báo cáo** 29°C","format":"markdown","auto_emphasis":true})).await.unwrap()["tin_id"],
        "63"
    );
    assert_eq!(
        post(&chat, "/api/messages/edit", "", json!({"chat_id":5,"message_id":62,"text":"*Sửa* 75%","format":"markdown","auto_emphasis":true})).await.unwrap()["id"],
        62
    );
    assert_eq!(
        post(&chat, "/api/send_media", "", json!({"peer":"7","media_b64":"b2s=","filename":"note.txt","caption":"**Ảnh** 29°C","format":"markdown","auto_emphasis":true})).await.unwrap()["tin_id"],
        "64"
    );
    assert_eq!(
        post(&chat, "/api/chats/5/send_file", "kind=photo&name=a.jpg&caption=**%E1%BA%A2nh**+29%C2%B0C&format=markdown&auto_emphasis=true", json!({"data_b64":"anBn"})).await.unwrap()["id"],
        65
    );
    assert_eq!(
        post(&chat, "/api/chats/5/send_album", "", json!({
            "format":"markdown",
            "auto_emphasis":true,
            "items":[
                {"kind":"photo","name":"p1.jpg","caption":"**Ảnh** 29°C","data_b64":"cDE="},
                {"kind":"photo","name":"p2.jpg","caption":"Đoạn 3 giờ","data_b64":"cDI="}
            ]
        })).await.unwrap()["count"],
        2
    );
}

#[tokio::test]
async fn limits_and_parameter_validation_enforce_4096_utf16_100_entities_and_reject_conflicts() {
    // 1. Đúng 4096 UTF-16 (2048 emoji 😀) được chấp nhận; 4098 UTF-16 (2049 emoji 😀) bị từ chối 400.
    let ok_4096 = "😀".repeat(2048);
    assert_eq!(utf16_len(&ok_4096), MAX_TEXT_UTF16);
    assert!(render_tdlib(&ok_4096, &plain_opts(false)).is_ok());
    assert!(render_tdlib(&format!("**{ok_4096}**"), &md_opts(false)).is_ok());

    let too_long_4098 = "😀".repeat(2049);
    assert_eq!(utf16_len(&too_long_4098), 4098);
    assert_eq!(render_tdlib(&too_long_4098, &plain_opts(false)).unwrap_err().status, 400);

    // 2. Giới hạn 100 entity (auto_emphasis và markdown).
    let many_units = "1 kg ".repeat(120);
    let r_auto = render(&many_units, &plain_opts(true)).unwrap();
    assert_eq!(r_auto.text, many_units);
    assert_eq!(r_auto.entities.len(), MAX_ENTITIES);

    let many_bold = "**a** ".repeat(120);
    let r_md = render(&many_bold, &md_opts(false)).unwrap();
    assert_eq!(r_md.text, "a ".repeat(120));
    assert_eq!(r_md.entities.len(), MAX_ENTITIES);

    // 3. Từ chối tham số sai kiểu, xung đột hoặc URL javascript: trong entities thủ công mà không gọi TDLib.
    let (chat, _) = setup(vec![]);
    for (path, query, body) in [
        ("/api/chats/5/send", "", json!({"text": too_long_4098})),
        ("/api/chats/5/send", "", json!({"text": format!("**{too_long_4098}**"), "format": "markdown"})),
        ("/api/chats/5/send", "", json!({"text": "x", "format": "html"})),
        ("/api/chats/5/send", "", json!({"text": "x", "format": 123})),
        ("/api/chats/5/send", "", json!({"text": "x", "auto_emphasis": "true"})),
        ("/api/chats/5/send", "", json!({"text": "x", "ai_markup": 99})),
        ("/api/chats/5/send", "", json!({"text": "x", "format": "markdown", "parse_mode": "html"})),
        ("/api/chats/5/send", "", json!({"text": "x", "auto_emphasis": true, "parse_mode": "markdown"})),
        ("/api/chats/5/send", "", json!({"text": "x", "format": "markdown", "entities": [{"type":"bold","offset":0,"length":1}]})),
        ("/api/chats/5/send", "", json!({"text": "x", "entities": [{"type":"text_link","offset":0,"length":1,"url":"javascript:alert(1)"}]})),
        ("/api/chats/5/send_file", "kind=photo&name=a.jpg&caption=29%C2%B0C&auto_emphasis=maybe", json!({"data_b64":"anBn"})),
        ("/api/messages/edit", "", json!({"chat_id":5,"message_id":1,"text":too_long_4098})),
        ("/api/send_message", "", json!({"peer":"7","message":"x","format":"xml"})),
    ] {
        let err = post(&chat, path, query, body.clone()).await.unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (400, "BAD_REQUEST"), "{path} {body}");
    }
    assert!(parse_options(&json!({"format":"markdown","auto_emphasis":true})).is_ok());

    // text_link thủ công hợp lệ (https:// và tg://) phải được gửi sang TDLib.
    let (chat_ok, _) = setup(vec![ok(
        json!({
            "@type":"sendMessage","chat_id":5,
            "input_message_content":{"@type":"inputMessageText","text":{
                "@type":"formattedText","text":"link",
                "entities":[{"@type":"textEntity","offset":0,"length":4,"type":{"@type":"textEntityTypeTextUrl","url":"tg://resolve?domain=demo"}}]
            },"link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}}
        }),
        td_message(70),
    )]);
    assert_eq!(
        post(&chat_ok, "/api/chats/5/send", "", json!({"text":"link","entities":[{"type":"text_link","offset":0,"length":4,"url":"tg://resolve?domain=demo"}]})).await.unwrap()["id"],
        70
    );
}

#[test]
fn mutation_counterexamples_catch_utf16_drift_missing_original_check_and_javascript_url() {
    // Đột biến 1: Offset UTF-16 bị lệch (nếu dùng chars().count()=2 hoặc bytes len()=5 thay vì UTF-16=3).
    let emoji_res = from_markdown("😀 **đậm**");
    assert_eq!(emoji_res.entities[0].offset, 3);
    assert_ne!(emoji_res.entities[0].offset, 2);
    assert_ne!(emoji_res.entities[0].offset, 5);

    let nfd_res = from_markdown("Chao\u{0300} **ba\u{0323}n**");
    assert_eq!((nfd_res.entities[0].offset, nfd_res.entities[0].length), (6, 4));
    assert_ne!((nfd_res.entities[0].offset, nfd_res.entities[0].length), (7, 5));

    // Đột biến 2: Bỏ kiểm chữ gốc ai_markup (from_markdown tự nó thành công, nhưng render bắt buộc 422).
    assert!(from_markdown("Nhiệt độ **30°C**").text == "Nhiệt độ 30°C");
    assert!(!preserves_original_text("Nhiệt độ 29°C", "Nhiệt độ 30°C"));
    let err = render("Nhiệt độ 29°C", &ai_opts("Nhiệt độ **30°C**", false)).unwrap_err();
    assert_eq!((err.status, err.code.as_str()), (422, "AI_MARKUP_MISMATCH"));

    // Đột biến 3: Chấp nhận URL javascript:.
    assert!(!is_allowed_url("javascript:alert(1)"));
    assert!(!is_allowed_url("JAVASCRIPT:alert(1)"));
    assert!(is_allowed_url("https://example.com"));
    assert!(is_allowed_url("tg://resolve?domain=demo"));
    let bad_md_link = from_markdown("[x](javascript:alert(1))");
    assert!(bad_md_link.entities.is_empty());

    // Đột biến 4: auto_emphasis đụng vùng mã.
    let mut code_entities = vec![ent(0, 4, EntityKind::Code)];
    apply_auto_emphasis("29°C và 75%", &mut code_entities);
    assert_eq!(
        code_entities,
        vec![ent(0, 4, EntityKind::Code), ent(8, 3, EntityKind::Bold)]
    );
}

#[tokio::test]
async fn rich_text_send_and_legacy_routes_enforce_auth_and_csrf() {
    let token = "token-gia-rich-text";
    let (chat, _) = setup(vec![
        ok(
            json!({
                "@type":"sendMessage","chat_id":5,
                "input_message_content":{"@type":"inputMessageText","text":{
                    "@type":"formattedText","text":"Cảnh báo 29°C",
                    "entities":[
                        {"@type":"textEntity","offset":0,"length":8,"type":{"@type":"textEntityTypeBold"}},
                        {"@type":"textEntity","offset":9,"length":4,"type":{"@type":"textEntityTypeBold"}}
                    ]
                },"link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}}
            }),
            td_message(80),
        ),
        ok(json!({"@type":"createPrivateChat","user_id":7,"force":false}), json!({"id":5})),
        ok(
            json!({
                "@type":"sendMessage","chat_id":5,
                "input_message_content":{"@type":"inputMessageText","text":{
                    "@type":"formattedText","text":"Mức 75%",
                    "entities":[{"@type":"textEntity","offset":4,"length":3,"type":{"@type":"textEntityTypeBold"}}]
                },"link_preview_options":{"@type":"linkPreviewOptions","is_disabled":true}}
            }),
            td_message(81),
        ),
    ]);
    let auth = Arc::new(UiAuth::in_memory(token));
    let grant = auth.setup(token, "nguoi-thu", "mat-khau-thu-dai", false).unwrap();
    let cookie = grant.cookie.split(';').next().unwrap().to_owned();
    let app = rest::router_with_ui_auth(
        Arc::new(rest::ChuaCo),
        Arc::new(chat),
        RestConfig {
            token: token.into(),
            rate_limit_per_minute: 120,
            version: "test".into(),
        },
        auth,
    );

    // 1. Không có thông tin xác thực hoặc sai Bearer -> 401.
    for path in ["/api/chats/5/send", "/api/send_message"] {
        let res_no_auth = app
            .clone()
            .oneshot(
                Request::post(path)
                    .body(Body::from(r#"{"text":"hi","format":"markdown"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res_no_auth.status(), 401, "{path}");

        let res_bad_bearer = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header(header::AUTHORIZATION, "Bearer sai-token")
                    .body(Body::from(r#"{"text":"hi","format":"markdown"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res_bad_bearer.status(), 401, "{path}");
    }

    // 2. Dùng cookie phiên nhưng thiếu CSRF / sai Origin -> 403.
    let res_no_csrf = app
        .clone()
        .oneshot(
            Request::post("/api/chats/5/send")
                .header(header::COOKIE, &cookie)
                .body(Body::from(r#"{"text":"hi","format":"markdown"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_no_csrf.status(), 403);

    let res_evil_origin = app
        .clone()
        .oneshot(
            Request::post("/api/chats/5/send")
                .header(header::COOKIE, &cookie)
                .header(header::HOST, "gateway.example")
                .header(header::ORIGIN, "http://evil.example")
                .body(Body::from(r#"{"text":"hi","format":"markdown"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_evil_origin.status(), 403);

    // 3. ai_markup sửa chữ gốc qua HTTP -> 422 AI_MARKUP_MISMATCH.
    let res_422 = app
        .clone()
        .oneshot(
            Request::post("/api/chats/5/send")
                .header(header::COOKIE, &cookie)
                .header("X-Requested-With", "fetch")
                .body(Body::from(
                    json!({"text":"Nhiệt độ 29°C","ai_markup":"Nhiệt độ **30°C**"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_422.status(), 422);
    let err_body: Value = serde_json::from_slice(&to_bytes(res_422.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(err_body["error"]["code"], "AI_MARKUP_MISMATCH");

    // 4. Cookie + CSRF và Bearer hợp lệ -> 200 OK.
    let res_cookie_ok = app
        .clone()
        .oneshot(
            Request::post("/api/chats/5/send")
                .header(header::COOKIE, &cookie)
                .header("X-Requested-With", "fetch")
                .body(Body::from(
                    json!({"text":"**Cảnh báo** 29°C","format":"markdown","auto_emphasis":true}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_cookie_ok.status(), 200);

    let res_bearer_ok = app
        .oneshot(
            Request::post("/api/send_message")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(json!({"peer":"7","message":"Mức 75%","auto_emphasis":true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_bearer_ok.status(), 200);
}


#[test]
fn oversized_raw_input_is_rejected_before_quadratic_parsing() {
    // B soát 22/11: 400.000 ký tự '*' từng tốn 40–240 s CPU rồi mới bị từ chối vì phân tích chạy trước phép kiểm độ dài.
    let started = std::time::Instant::now();
    for bomb in ["*".repeat(400_000), "[a](".repeat(100_000), "😀".repeat(MAX_RAW_UTF16)] {
        for opts in [md_opts(true), md_opts(false), plain_opts(true)] {
            assert_eq!(render(&bomb, &opts).unwrap_err().status, 400);
        }
        assert_eq!(render("ok", &ai_opts(&bomb, false)).unwrap_err().status, 400);
        assert_eq!(render(&bomb, &ai_opts("ok", false)).unwrap_err().status, 400);
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "từ chối phải tức thì, mất {:?}", started.elapsed());
    // Đúng ở biên: đầu vào thô đúng trần vẫn được phân tích (chữ sau khi bỏ dấu còn trong 4096 thì nhận).
    let at_limit = format!("{}{}", "**a**".repeat(60), "b".repeat(MAX_RAW_UTF16 - 300));
    assert_eq!(utf16_len(&at_limit), MAX_RAW_UTF16);
    assert_eq!(render(&at_limit, &md_opts(false)).unwrap_err().status, 400, "chữ dựng xong vẫn > 4096 nên bị từ chối ở phép kiểm cũ");
    let fits = format!("{}{}", "**a**".repeat(60), "b".repeat(3000));
    assert!(render(&fits, &md_opts(false)).is_ok());
}

#[test]
fn from_markdown_handles_empty_fenced_blocks_and_four_backtick_runs() {
    let r = from_markdown("```\n```\nNormal\n```rust\nlet x = 1;\n```");
    assert!(r.text.contains("Normal\n"));
    let expected_offset = utf16_len("```\n```\nNormal\n");
    assert!(r
        .entities
        .contains(&ent(expected_offset, 10, EntityKind::Pre(Some("rust".to_owned())))));
    assert!(!r
        .entities
        .iter()
        .any(|e| matches!(e.kind, EntityKind::Pre(_)) && e.offset < expected_offset));

    let q = from_markdown("````\n> quoted\n`");
    assert_eq!(
        q.entities,
        vec![ent(utf16_len("````\n"), utf16_len("quoted"), EntityKind::BlockQuote)]
    );
}

#[test]
fn ai_markup_blocks_zwj_flood_and_skin_tone_mutation_and_allows_shared_prefix_emoji() {
    let flood = format!("OK {}", vec!["😀"; 50].join("\u{200D}"));
    assert!(!preserves_original_text("OK", &flood));
    assert_eq!(render("OK", &ai_opts(&flood, false)).unwrap_err().status, 422);
    assert!(!preserves_original_text("👍 OK", "👍\u{1F3FB} OK"));
    assert!(!preserves_original_text("OK", "OK \u{1F3FB}"));
    assert!(preserves_original_text("🇻🇳 Hà Nội", "🇻🇦 🇻🇳 Hà Nội"));
    assert!(preserves_original_text("👨 Bố", "👨\u{200D}💻 👨 Bố"));
}

#[test]
fn auto_emphasis_matches_plural_hrs_mins_secs() {
    let r = render("Còn 5 hrs, 10 mins, 30 secs.", &plain_opts(true)).unwrap();
    assert_eq!(
        r.entities,
        vec![
            ent(4, 5, EntityKind::Bold),
            ent(11, 7, EntityKind::Bold),
            ent(20, 7, EntityKind::Bold),
        ]
    );
}
