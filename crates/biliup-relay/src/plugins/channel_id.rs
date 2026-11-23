//! 平台频道 id 的规范化：Twitch 一律小写，其余去掉首尾空白。

pub(crate) fn canonical_channel_id(platform: &str, channel_id: &str) -> String {
    if platform == "TW" {
        channel_id.trim().to_ascii_lowercase()
    } else {
        channel_id.trim().to_owned()
    }
}
