/// 交错（interleaved）音频缓冲。
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    channels: usize,
    frames: usize,
    data: Vec<f32>,
}

impl AudioBuffer {
    /// 创建静音缓冲：`channels` 个通道、`frames` 帧。
    pub fn silence(channels: usize, frames: usize) -> Self {
        AudioBuffer { channels, frames, data: vec![0.0; channels * frames] }
    }

    /// 通道数。
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// 帧数（每通道的采样数）。
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// 交错采样数据的只读视图（长度 = `channels * frames`）。
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// 交错采样数据的可变视图（长度 = `channels * frames`）。
    pub fn data_mut(&mut self) -> &mut [f32] {
        &mut self.data
    }

    /// 读取指定通道、指定帧的采样（索引基于 `frame * channels + channel`）。
    pub fn sample(&self, channel: usize, frame: usize) -> f32 {
        self.data[frame * self.channels + channel]
    }

    /// 写入指定通道、指定帧的采样（索引基于 `frame * channels + channel`）。
    pub fn set_sample(&mut self, channel: usize, frame: usize, value: f32) {
        self.data[frame * self.channels + channel] = value;
    }

    /// 所有采样的绝对峰值。
    pub fn peak(&self) -> f32 {
        self.data.iter().fold(0.0f32, |a, b| a.max(b.abs()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_allocates_zeroed_interleaved_data() {
        let buf = AudioBuffer::silence(2, 4);
        assert_eq!(buf.channels(), 2);
        assert_eq!(buf.frames(), 4);
        assert_eq!(buf.data().len(), 8);
        assert!(buf.peak() == 0.0);
    }

    #[test]
    fn sample_and_set_sample_are_interleaved() {
        let mut buf = AudioBuffer::silence(2, 3);
        buf.set_sample(1, 2, -0.5);
        assert_eq!(buf.data()[2 * 2 + 1], -0.5);
        assert_eq!(buf.sample(1, 2), -0.5);
        assert_eq!(buf.sample(0, 2), 0.0);
        assert_eq!(buf.peak(), 0.5);
        buf.data_mut()[0] = 1.0;
        assert_eq!(buf.sample(0, 0), 1.0);
    }
}
