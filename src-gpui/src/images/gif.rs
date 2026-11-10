//! GIF 顺序解码与帧合成；只持有压缩源和当前画布，不保存历史帧。
use super::{Failure, MAX_ANIMATION_BYTES, MAX_BITMAP_PIXELS, MAX_DECODE_BYTES, MAX_DECODE_WIDTH, MAX_INPUT_PIXELS};
use image::{AnimationDecoder, Frame, ImageDecoder, imageops::FilterType};
use std::{io::Cursor, num::NonZeroU64, sync::Arc};

pub(super) struct DecodedGif {
    pub first: Frame,
    pub width: u32,
    pub height: u32,
    pub stream: Option<GifStream>,
    pub notice: Option<Failure>,
}

pub(super) struct GifStream {
    encoded: Arc<[u8]>,
    reader: gif::Decoder<Cursor<Arc<[u8]>>>,
    screen: gif_dispose::Screen,
    output: (u32, u32),
    frames_in_loop: u32,
    repetitions: Option<gif::Repeat>,
    /// 最坏同时存活的位图、索引缓冲和缩放临时结果；压缩源另计。
    pub working_bytes: usize,
}

fn decode_error(error: gif::DecodingError) -> Failure {
    match error {
        gif::DecodingError::MemoryLimit | gif::DecodingError::OutOfMemory => Failure::Budget,
        _ => Failure::Decode,
    }
}

fn reader(encoded: Arc<[u8]>) -> Result<gif::Decoder<Cursor<Arc<[u8]>>>, Failure> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    // gif-dispose 的矩形合成要求帧不能越出逻辑画布，必须在分配／合成前拒绝。
    options.check_frame_consistency(true);
    options.set_memory_limit(gif::MemoryLimit::Bytes(NonZeroU64::new(MAX_INPUT_PIXELS).unwrap()));
    options.read_info(Cursor::new(encoded)).map_err(decode_error)
}

fn output_size(width: u32, height: u32) -> (u32, u32) {
    let pixels = u64::from(width) * u64::from(height);
    let scale = (MAX_DECODE_WIDTH as f64 / width.max(height) as f64)
        .min((MAX_BITMAP_PIXELS as f64 / pixels as f64).sqrt()).min(1.0);
    ((width as f64 * scale).max(1.0) as u32, (height as f64 * scale).max(1.0) as u32)
}

pub(super) fn decode(encoded: Vec<u8>) -> Result<DecodedGif, Failure> {
    let encoded: Arc<[u8]> = encoded.into();
    let decoder = reader(encoded.clone())?;
    let (width, height) = (u32::from(decoder.width()), u32::from(decoder.height()));
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_INPUT_PIXELS || width > 32_768 || height > 32_768 {
        return Err(Failure::Budget);
    }
    let output = output_size(width, height);
    let native_bytes = pixels as usize * 4;
    let frame_bytes = output.0 as usize * output.1 as usize * 4;
    // 画布、旧／新 Previous 恢复块、索引缓冲，以及首帧／当前／两帧预读／待回收帧。
    // resize 的浮点中间行另作保守预留，不能只按缩小后的像素数决定是否允许播放。
    let resize_bytes = if output == (width, height) { 0 } else { native_bytes * 5 };
    let working_bytes = native_bytes * 3 + pixels as usize * 2 + frame_bytes * 5 + resize_bytes + 256 * 1024;
    if working_bytes > MAX_ANIMATION_BYTES {
        drop(decoder);
        return Ok(DecodedGif {
            first: first_frame(encoded, output)?, width, height, stream: None, notice: Some(Failure::Budget),
        });
    }
    let screen = gif_dispose::Screen::new_decoder(&decoder);
    let mut stream = GifStream {
        encoded, reader: decoder, screen, output, frames_in_loop: 0, repetitions: None, working_bytes,
    };
    let first = stream.next_frame()?.ok_or(Failure::Decode)?;
    Ok(DecodedGif { first, width, height, stream: Some(stream), notice: None })
}

impl GifStream {
    pub fn reserved_bytes(&self) -> usize { self.working_bytes + self.encoded.len() + 4096 }

    pub fn next_frame(&mut self) -> Result<Option<Frame>, Failure> {
        // 一次调用最多跨越一次循环边界；空流／单帧 GIF 不产生忙循环。
        for _ in 0..2 {
            if let Some(frame) = self.reader.read_next_frame().map_err(decode_error)? {
                if frame.width == 0 || frame.height == 0 { return Err(Failure::Decode); }
                let delay = image::Delay::from_numer_denom_ms((u32::from(frame.delay) * 10).max(20), 1);
                self.screen.blit_frame(frame).map_err(|_| Failure::Decode)?;
                self.frames_in_loop = self.frames_in_loop.saturating_add(1);
                let pixels = self.screen.pixels_rgba();
                // 直接生成 BGRA，避免另留一份 RGBA 全画布。
                let mut buffer = image::RgbaImage::from_fn(pixels.width() as u32, pixels.height() as u32, |x, y| {
                    let p = pixels.buf()[y as usize * pixels.stride() + x as usize];
                    image::Rgba([p.b, p.g, p.r, p.a])
                });
                if buffer.dimensions() != self.output {
                    buffer = image::imageops::resize(&buffer, self.output.0, self.output.1, FilterType::Triangle);
                }
                return Ok(Some(Frame::from_parts(buffer, 0, 0, delay)));
            }
            if self.frames_in_loop < 2 { return Ok(None); }
            let repetitions = self.repetitions.get_or_insert_with(|| self.reader.repeat());
            match repetitions {
                gif::Repeat::Finite(0) => return Ok(None),
                gif::Repeat::Finite(count) => *count -= 1,
                gif::Repeat::Infinite => {},
            }
            self.reader = reader(self.encoded.clone())?;
            // 不把上一轮末帧带到下一轮，首帧可能只覆盖局部或包含透明像素。
            self.screen = gif_dispose::Screen::new_decoder(&self.reader);
            self.frames_in_loop = 0;
        }
        Err(Failure::Decode)
    }
}

/// 动画工作集过大时仍尝试已有的有界首帧解码，不先分配整段动画。
fn first_frame(encoded: Arc<[u8]>, output: (u32, u32)) -> Result<Frame, Failure> {
    let error = |error| if matches!(error, image::ImageError::Limits(_)) { Failure::Budget } else { Failure::Decode };
    let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(encoded)).map_err(error)?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    decoder.set_limits(limits).map_err(error)?;
    let frame = decoder.into_frames().next().ok_or(Failure::Decode)?.map_err(error)?;
    let mut buffer = frame.into_buffer();
    if buffer.dimensions() != output { buffer = image::imageops::resize(&buffer, output.0, output.1, FilterType::Triangle); }
    for pixel in buffer.chunks_exact_mut(4) { pixel.swap(0, 2); }
    Ok(Frame::new(buffer))
}
