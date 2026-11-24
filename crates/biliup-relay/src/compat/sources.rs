#![allow(dead_code)]
//! 尚未迁入的 C 组（YouTube/Twitch/Niconico）被 B 组跨引用的薄桩。
//!
//! 待 P1-03 迁入对应插件后删除。网络拉取类函数一律返回明确错误，不发外网请求；
//! Niconico 的频道身份归一与 UA 是纯逻辑/常量，保持与源仓库一致。

use crate::config::Niconico as NiconicoConfig;

pub mod twitch {
    pub type TwitchGqlStatus = (bool, Option<String>, Option<String>, Option<String>);

    /// 待 P1-03 Twitch 插件迁入后替换；本阶段不访问外网。
    pub async fn get_twitch_status(
        _channel_id: &str,
    ) -> Result<TwitchGqlStatus, Box<dyn std::error::Error>> {
        Err("Twitch 状态查询待 P1-03 迁入".into())
    }
}

pub mod youtube {
    use crate::plugins::source_status::SourceStatus;

    /// 待 P1-03 YouTube 插件迁入后替换；本阶段不访问外网。
    pub async fn get_youtube_status(
        _channel_id: &str,
    ) -> Result<SourceStatus, Box<dyn std::error::Error>> {
        Err("YouTube 状态查询待 P1-03 迁入".into())
    }

    /// 待 P1-03 YouTube 分区建议迁入后替换。
    pub async fn get_youtube_area_topic(
        _channel_id: &str,
        _video_id: Option<&str>,
    ) -> Option<String> {
        None
    }
}

pub mod niconico {
    use super::NiconicoConfig;

    pub const CHANNEL_PAGE_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

    /// 待 P1-03 Niconico 插件迁入后替换；本阶段不访问外网。
    pub async fn niconico_cover_thumbnail_url(
        _live_id: &str,
        _proxy: Option<&str>,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        Err("Niconico 封面查询待 P1-03 迁入".into())
    }

    pub fn normalize_channel_id(input: &str) -> String {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.contains("live.nicovideo.jp/watch/") {
            return String::new();
        }
        let without_host = trimmed
            .strip_prefix("https://ch.nicovideo.jp/")
            .or_else(|| trimmed.strip_prefix("http://ch.nicovideo.jp/"))
            .or_else(|| trimmed.strip_prefix("https://www.nicovideo.jp/"))
            .unwrap_or(trimmed)
            .trim_start_matches('/');
        let first = without_host
            .split(['/', '?', '#'])
            .find(|part| !part.is_empty())
            .unwrap_or("");
        if first.is_empty() || first == "watch" || first.starts_with("lv") || first.contains(':') {
            return String::new();
        }
        first.to_string()
    }

    fn restream_name(channel: &crate::config::Channel) -> String {
        channel
            .niconico_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(channel.name.trim())
            .to_string()
    }

    fn name_from_channels(channels: &[crate::config::Channel], channel_id: &str) -> Option<String> {
        let id = normalize_channel_id(channel_id);
        if id.is_empty() {
            return None;
        }
        channels.iter().find_map(|channel| {
            let nico_id = channel.platforms.niconico.as_deref()?;
            (normalize_channel_id(nico_id) == id).then(|| restream_name(channel))
        })
    }

    pub fn niconico_channel_identity(cfg: &NiconicoConfig) -> (String, String) {
        let id = normalize_channel_id(&cfg.channel_id);
        let name = if id.is_empty() {
            cfg.channel_name.trim().to_string()
        } else {
            crate::config::load_channels()
                .ok()
                .and_then(|channels| name_from_channels(&channels.channels, &id))
                .unwrap_or_else(|| cfg.channel_name.trim().to_string())
        };
        (id, name)
    }
}

pub use niconico::niconico_channel_identity;
