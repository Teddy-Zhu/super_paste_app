use super::layout::{LayoutItem, arrange};
use super::models::InstalledModel;
use image::{ImageReader, Limits, RgbImage};
use oar_ocr::{
    core::{
        config::{ModelInferenceConfig, OrtSessionConfig},
        inference::OrtInfer,
    },
    models::recognition::CRNNModel,
    predictors::TextDetectionPredictor,
    processors::{CTCLabelDecode, OCRResize},
    utils::get_rotate_crop_image,
};
use std::sync::OnceLock;
use std::{
    io::Cursor,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;
const MAX_PIXELS: u64 = 40_000_000;
const MAX_IMAGE_EDGE: u32 = 65_536;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DETECTIONS: usize = 20_000;
const MAX_DICTIONARY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DICTIONARY_TOKENS: usize = 200_000;
const MAX_LINE_WIDTH: usize = 3_072;

#[derive(Clone, Debug)]
pub(super) struct LocalRecognition {
    pub text: String,
    pub confidence: f64,
}

pub(super) struct LocalWorker {
    normal: Sender<LocalCommand>,
    urgent: Sender<LocalCommand>,
    thread: std::sync::Mutex<Option<JoinHandle<()>>>,
    loaded_model: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

enum LocalCommand {
    Recognize {
        model: InstalledModel,
        image: Vec<u8>,
        cancel: CancellationToken,
        result: oneshot::Sender<Result<LocalRecognition, String>>,
    },
    Unload {
        model_id: Option<String>,
        result: oneshot::Sender<()>,
    },
    Shutdown,
}

impl LocalWorker {
    pub(super) fn new(ort_runtime: Result<PathBuf, String>) -> Result<Self, String> {
        let (normal_tx, normal_rx) = mpsc::channel();
        let (urgent_tx, urgent_rx) = mpsc::channel();
        let loaded_model = std::sync::Arc::new(std::sync::Mutex::new(None));
        let loaded_state = loaded_model.clone();
        let worker = thread::Builder::new()
            .name("ocr-local-inference".to_owned())
            .spawn(move || local_worker_loop(normal_rx, urgent_rx, loaded_state, ort_runtime))
            .map_err(|error| format!("无法启动本地 OCR 工作线程: {error}"))?;
        Ok(Self {
            normal: normal_tx,
            urgent: urgent_tx,
            thread: std::sync::Mutex::new(Some(worker)),
            loaded_model,
        })
    }

    pub(super) fn is_loaded(&self, model_id: &str) -> bool {
        self.loaded_model
            .lock()
            .map(|loaded| loaded.as_deref() == Some(model_id))
            .unwrap_or(false)
    }

    pub(super) async fn recognize(
        &self,
        model: InstalledModel,
        image: Vec<u8>,
        cancel: CancellationToken,
        urgent: bool,
    ) -> Result<LocalRecognition, String> {
        let (result, receiver) = oneshot::channel();
        let command = LocalCommand::Recognize {
            model,
            image,
            cancel,
            result,
        };
        if urgent {
            self.urgent
                .send(command)
                .map_err(|_| "本地 OCR 工作线程已关闭".to_owned())?;
        } else {
            self.normal
                .send(command)
                .map_err(|_| "本地 OCR 工作线程已关闭".to_owned())?;
        }
        receiver
            .await
            .map_err(|_| "本地 OCR 任务异常退出".to_owned())?
    }

    pub(super) async fn unload(&self, model_id: Option<String>) -> Result<(), String> {
        let (result, receiver) = oneshot::channel();
        self.urgent
            .send(LocalCommand::Unload { model_id, result })
            .map_err(|_| "本地 OCR 工作线程已关闭".to_owned())?;
        receiver
            .await
            .map_err(|_| "本地 OCR 工作线程异常退出".to_owned())
    }

    pub(super) async fn shutdown(&self) -> Result<(), String> {
        let Some(thread) = self
            .thread
            .lock()
            .map_err(|_| "本地 OCR 工作线程状态不可用".to_owned())?
            .take()
        else {
            return Ok(());
        };
        let requested = self
            .urgent
            .send(LocalCommand::Shutdown)
            .map_err(|_| "本地 OCR 工作线程已关闭".to_owned());
        tokio::task::spawn_blocking(move || thread.join())
            .await
            .map_err(|error| format!("等待本地 OCR 工作线程失败: {error}"))?
            .map_err(|_| "本地 OCR 工作线程崩溃".to_owned())?;
        requested
    }
}

fn local_worker_loop(
    normal: Receiver<LocalCommand>,
    urgent: Receiver<LocalCommand>,
    loaded_model: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    ort_runtime: Result<PathBuf, String>,
) {
    let mut engine: Option<LocalEngine> = None;
    loop {
        let command = match urgent.try_recv() {
            Ok(command) => Some(command),
            Err(TryRecvError::Disconnected) => normal.recv().ok(),
            Err(TryRecvError::Empty) => match normal.recv_timeout(Duration::from_millis(8)) {
                Ok(command) => Some(command),
                // An idle queue is still open; only disconnection ends the worker.
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => urgent.recv().ok(),
            },
        };
        let Some(command) = command else {
            break;
        };
        match command {
            LocalCommand::Recognize {
                model,
                image,
                cancel,
                result,
            } => {
                let outcome = (|| {
                    check_cancelled(&cancel)?;
                    if engine
                        .as_ref()
                        .is_none_or(|loaded| loaded.model_id != model.info.id)
                    {
                        engine = None;
                        ensure_ort_runtime(&ort_runtime)?;
                        engine = Some(LocalEngine::load(&model, &cancel)?);
                        if let Ok(mut loaded) = loaded_model.lock() {
                            *loaded = Some(model.info.id.clone());
                        }
                    }
                    engine
                        .as_ref()
                        .ok_or_else(|| "本地 OCR 引擎未初始化".to_owned())?
                        .recognize(&image, &cancel)
                })();
                let _ = result.send(outcome);
            }
            LocalCommand::Unload { model_id, result } => {
                if model_id.as_deref().is_none_or(|requested| {
                    engine
                        .as_ref()
                        .is_some_and(|loaded| loaded.model_id == requested)
                }) {
                    engine = None;
                    if let Ok(mut loaded) = loaded_model.lock() {
                        *loaded = None;
                    }
                }
                let _ = result.send(());
            }
            LocalCommand::Shutdown => {
                if let Ok(mut loaded) = loaded_model.lock() {
                    *loaded = None;
                }
                break;
            }
        }
    }
}

fn ensure_ort_runtime(runtime: &Result<PathBuf, String>) -> Result<(), String> {
    static ORT_INIT: OnceLock<Result<(), String>> = OnceLock::new();
    let path = runtime
        .as_ref()
        .map_err(|error| format!("本地 OCR 运行时不可用: {error}"))?;
    if !path.is_absolute() || !path.is_file() {
        return Err("打包的 ONNX Runtime 动态库路径无效".to_owned());
    }
    ORT_INIT
        .get_or_init(|| {
            ort::init_from(path)
                .map_err(|error| format!("加载 ONNX Runtime 动态库失败: {error}"))?
                .commit();
            Ok(())
        })
        .clone()
}

struct LocalEngine {
    model_id: String,
    optimize_space: bool,
    detector: TextDetectionPredictor,
    recognizer: CRNNModel,
    dictionary: Vec<String>,
}

impl LocalEngine {
    fn load(model: &InstalledModel, cancel: &CancellationToken) -> Result<Self, String> {
        check_cancelled(cancel)?;
        let metadata = std::fs::metadata(&model.dictionary)
            .map_err(|error| format!("无法读取 OCR 字典: {error}"))?;
        if metadata.len() > MAX_DICTIONARY_BYTES {
            return Err("OCR 字典超过安全大小限制".to_owned());
        }
        let source = std::fs::read_to_string(&model.dictionary)
            .map_err(|error| format!("无法读取 OCR 字典: {error}"))?;
        let source = source.strip_prefix('\u{feff}').unwrap_or(&source);
        if source.trim().is_empty() {
            return Err("OCR 字典为空".to_owned());
        }
        let mut dictionary = source
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .split('\n')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if dictionary.last().is_some_and(String::is_empty) {
            dictionary.pop();
        }
        if dictionary.last().is_none_or(|token| token != " ") {
            dictionary.push(" ".to_owned());
        }
        if dictionary.len() > MAX_DICTIONARY_TOKENS {
            return Err("OCR 字典字符数超过安全上限".to_owned());
        }
        let session = OrtSessionConfig::new()
            .with_intra_threads(2)
            .with_inter_threads(1)
            .with_parallel_execution(false);
        let detector = TextDetectionPredictor::builder()
            .score_threshold(0.3)
            .box_threshold(0.6)
            .unclip_ratio(1.5)
            .max_candidates(MAX_DETECTIONS)
            .with_ort_config(session.clone())
            .build(&model.detector)
            .map_err(|error| format!("加载 OCR 文本检测模型失败: {error}"))?;
        check_cancelled(cancel)?;
        let inference = OrtInfer::from_config(
            &ModelInferenceConfig {
                ort_session: Some(session),
                ..Default::default()
            },
            &model.recognizer,
            None,
        )
        .map_err(|error| format!("加载 OCR 文本识别模型失败: {error}"))?;
        let shape = inference
            .primary_input_shape()
            .ok_or_else(|| "OCR 识别模型缺少输入形状".to_owned())?;
        if shape.len() != 4 || (shape[1] > 0 && shape[1] != 3) {
            return Err("不支持的 OCR 识别模型输入布局".to_owned());
        }
        let recognition_height = if shape[2] > 0 {
            usize::try_from(shape[2]).map_err(|_| "OCR 模型输入高度无效".to_owned())?
        } else {
            48
        };
        let recognition_width = if shape[3] > 0 {
            usize::try_from(shape[3]).map_err(|_| "OCR 模型输入宽度无效".to_owned())?
        } else {
            320
        };
        if !(16..=128).contains(&recognition_height) || recognition_width > MAX_LINE_WIDTH {
            return Err("OCR 识别模型输入尺寸超出支持范围".to_owned());
        }
        let resizer = OCRResize::with_max_width(
            Some([3, recognition_height, recognition_width]),
            None,
            Some(MAX_LINE_WIDTH),
        );
        let decoder = CTCLabelDecode::from_string_list(Some(&dictionary), false, false);
        let recognizer = CRNNModel::new(inference, resizer, decoder);
        check_cancelled(cancel)?;
        Ok(Self {
            model_id: model.info.id.clone(),
            optimize_space: model.info.optimize_space,
            detector,
            recognizer,
            dictionary,
        })
    }

    fn recognize(
        &self,
        encoded: &[u8],
        cancel: &CancellationToken,
    ) -> Result<LocalRecognition, String> {
        check_cancelled(cancel)?;
        let image = std::sync::Arc::new(decode_image(encoded)?);
        let detected = self
            .detector
            .predict_shared(vec![image.clone()])
            .map_err(|error| format!("OCR 图像检测失败: {error}"))?;
        let boxes = detected
            .detections
            .into_iter()
            .next()
            .ok_or_else(|| "OCR 检测器没有返回图像结果".to_owned())?
            .into_iter()
            .map(|detection| detection.bbox)
            .collect::<Vec<_>>();
        if boxes.len() > MAX_DETECTIONS {
            return Err("OCR 文本区域超过安全上限".to_owned());
        }
        let mut lines = Vec::with_capacity(boxes.len());
        for bounds in boxes {
            check_cancelled(cancel)?;
            if bounds.points.len() != 4
                || bounds
                    .points
                    .iter()
                    .any(|point| !point.x.is_finite() || !point.y.is_finite())
            {
                continue;
            }
            let crop = get_rotate_crop_image(&image, &bounds.points)
                .map_err(|error| format!("裁剪 OCR 文本行失败: {error}"))?;
            let (text, confidence) = self.recognize_line(&crop, cancel)?;
            if confidence >= 0.5 && !text.trim().is_empty() {
                lines.push(LayoutItem {
                    points: [
                        [bounds.points[0].x, bounds.points[0].y],
                        [bounds.points[1].x, bounds.points[1].y],
                        [bounds.points[2].x, bounds.points[2].y],
                        [bounds.points[3].x, bounds.points[3].y],
                    ],
                    text: text.trim().to_owned(),
                    confidence,
                });
            }
        }
        check_cancelled(cancel)?;
        let (text, confidence) = arrange(lines);
        Ok(LocalRecognition {
            text,
            confidence: (confidence * 100.0).round() / 100.0,
        })
    }

    fn recognize_line(
        &self,
        crop: &RgbImage,
        cancel: &CancellationToken,
    ) -> Result<(String, f64), String> {
        check_cancelled(cancel)?;
        if crop.width() == 0 || crop.height() == 0 {
            return Ok((String::new(), 0.0));
        }
        let tensor = self
            .recognizer
            .preprocess_refs(&[crop])
            .map_err(|error| format!("OCR 图像预处理失败: {error}"))?;
        let output = self
            .recognizer
            .infer(&tensor)
            .map_err(|error| format!("OCR 文本识别失败: {error}"))?;
        let shape = output.shape();
        if shape.len() != 3
            || shape[0] != 1
            || shape[1] == 0
            || shape[2] != self.dictionary.len() + 1
        {
            return Err("OCR 识别模型输出与字典不匹配".to_owned());
        }
        let values = output
            .as_slice()
            .ok_or_else(|| "OCR 识别模型输出不可读取".to_owned())?;
        let mut text = String::new();
        let mut previous = None;
        let mut score_sum = 0.0f64;
        let mut score_count = 0usize;
        let vocab_size = shape[2];
        for row in values.chunks_exact(vocab_size) {
            check_cancelled(cancel)?;
            let mut best: Option<(usize, f32)> = None;
            let mut second: Option<(usize, f32)> = None;
            for (index, score) in row.iter().copied().enumerate() {
                if !score.is_finite() || score < 1e-5 {
                    continue;
                }
                if best.is_none_or(|(_, best_score)| score >= best_score) {
                    second = best;
                    best = Some((index, score));
                } else if second.is_none_or(|(_, second_score)| score >= second_score) {
                    second = Some((index, score));
                }
            }
            let Some(best) = best else {
                previous = None;
                continue;
            };
            if best.0 != 0 && previous != Some(best.0) {
                let token_for = |index: usize| {
                    if index == 0 {
                        ""
                    } else {
                        self.dictionary
                            .get(index - 1)
                            .map(String::as_str)
                            .unwrap_or("")
                    }
                };
                let mut selected = best;
                if self.optimize_space && token_for(best.0).is_empty() {
                    if let Some(candidate) = second
                        .filter(|candidate| token_for(candidate.0) == " " && candidate.1 > 1e-3)
                    {
                        selected = candidate;
                    }
                }
                text.push_str(token_for(selected.0));
                score_sum += f64::from(selected.1);
                score_count += 1;
            }
            previous = Some(best.0);
        }
        let confidence = if score_count == 0 {
            0.0
        } else {
            score_sum / score_count as f64
        };
        Ok((text.trim().to_owned(), confidence))
    }
}

fn decode_image(encoded: &[u8]) -> Result<RgbImage, String> {
    if encoded.is_empty() {
        return Err("图片数据为空".to_owned());
    }
    if encoded.len() > MAX_ENCODED_BYTES {
        return Err("OCR 图片超过识别上限".to_owned());
    }
    let mut reader = ImageReader::new(Cursor::new(encoded))
        .with_guessed_format()
        .map_err(|error| format!("无法识别图片格式: {error}"))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("图片解码失败: {error}"))?;
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("OCR 图片像素超过识别上限".to_owned());
    }
    Ok(image.to_rgb8())
}

fn check_cancelled(cancel: &CancellationToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("OCR 任务已取消".to_owned())
    } else {
        Ok(())
    }
}
