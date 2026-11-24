#![allow(dead_code)]
//! 阶段替换桩：niconico 只借 PipedIngest 这个数据形状，真正的 ffmpeg 管道由 P1-04 D 组迁入。
//!
//! 待 P1-04 ffmpeg 插件迁入后，本文件整体由真实的 plugins/ffmpeg.rs 取代，
//! 形状与 bilistream/src/plugins/ffmpeg.rs 的 PipedIngest 一致，调用方无需改动。

/// 预复用成 MPEG-TS 的生产者（streamlink nicolive）被管道送入 ffmpeg。
#[derive(Clone, Debug)]
pub struct PipedIngest {
    pub program: String,
    pub args: Vec<String>,
}
