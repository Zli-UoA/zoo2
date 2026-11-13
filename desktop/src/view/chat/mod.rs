//! チャット領域 (frontend の `components/chat/**` に対応)。

mod attachments;
mod message;
mod ogp_card;
mod thread;
mod timeline;

pub use attachments::{format_file_size, render_attachment_item};
pub use ogp_card::{
    CARD_DESC_MAX_CHARS, CARD_DESC_MAX_LINES, is_ogp_description_long, render_ogp_card,
    truncate_ogp_description,
};
