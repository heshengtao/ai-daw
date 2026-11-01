//! Manual diagnostic: **run a real Conv on CUDA** (not just "the session builds").
//!
//! Why a separate test: cuDNN 9.x splits the implementation into several `cudnn_*.dll`
//! sublibraries. Creating a session does not load them; the first Conv makes the cuDNN
//! frontend `LoadLibrary` them by bare name. If a sublibrary is missing, the session
//! builds fine and the failure only appears halfway through a real task.
//!
//! Run it:
//!
//! ```text
//! $env:AIDAW_TEST_RUNTIME="$env:APPDATA\com.aidaw.app\runtime\ort\cuda"
//! cargo test -p ai-daw-local-ai --test cudnn_conv -- --nocapture
//! ```
//!
//! Skips on non-Windows builds (the diagnostic was written for the Windows cuDNN layout).
#![cfg(windows)]

use std::path::PathBuf;

/// Minimal ONNX protobuf writer -- avoids pulling a protobuf crate just for one test.
mod onnx {
    fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn key(field: u32, wire: u32) -> Vec<u8> {
        varint(((field as u64) << 3) | wire as u64)
    }

    fn varint_field(field: u32, value: u64, out: &mut Vec<u8>) {
        out.extend(key(field, 0));
        out.extend(varint(value));
    }

    fn bytes_field(field: u32, bytes: &[u8], out: &mut Vec<u8>) {
        out.extend(key(field, 2));
        out.extend(varint(bytes.len() as u64));
        out.extend(bytes);
    }

    fn string_field(field: u32, text: &str, out: &mut Vec<u8>) {
        bytes_field(field, text.as_bytes(), out);
    }

    fn dim(value: i64) -> Vec<u8> {
        let mut out = Vec::new();
        varint_field(1, value as u64, &mut out);
        out
    }

    fn tensor(name: &str, dims: &[i64], data: &[f32]) -> Vec<u8> {
        let mut out = Vec::new();
        for d in dims {
            varint_field(1, *d as u64, &mut out);
        }
        varint_field(2, 1, &mut out); // TensorProto.DataType.FLOAT
        string_field(8, name, &mut out);
        let raw: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        bytes_field(9, &raw, &mut out);
        out
    }

    fn value_info(name: &str, dims: &[i64]) -> Vec<u8> {
        let mut tensor_type = Vec::new();
        varint_field(1, 1, &mut tensor_type); // elem_type = FLOAT
        let mut shape = Vec::new();
        for d in dims {
            bytes_field(1, &dim(*d), &mut shape);
        }
        bytes_field(2, &shape, &mut tensor_type);
        let mut type_proto = Vec::new();
        bytes_field(1, &tensor_type, &mut type_proto);
        let mut out = Vec::new();
        string_field(1, name, &mut out);
        bytes_field(2, &type_proto, &mut out);
        out
    }

    fn attribute_ints(name: &str, ints: &[i64]) -> Vec<u8> {
        let mut out = Vec::new();
        string_field(1, name, &mut out);
        varint_field(20, 7, &mut out); // AttributeProto.AttributeType.INTS
        for i in ints {
            varint_field(8, *i as u64, &mut out);
        }
        out
    }

    fn node(inputs: &[&str], outputs: &[&str], name: &str, op_type: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for input in inputs {
            string_field(1, input, &mut out);
        }
        for output in outputs {
            string_field(2, output, &mut out);
        }
        string_field(3, name, &mut out);
        string_field(4, op_type, &mut out);
        for (attr, values) in [
            ("strides", &[480i64][..]),
            ("pads", &[0i64, 0][..]),
            ("dilations", &[1i64, 1][..]),
            ("group", &[1i64][..]),
            ("kernel_shape", &[1920i64][..]),
        ] {
            bytes_field(5, &attribute_ints(attr, values), &mut out);
        }
        out
    }

    /// Build a one-node Conv model with a fixed pseudo-random kernel.
    pub fn conv_model(input: &[i64], weight: &[i64], output: &[i64]) -> Vec<u8> {
        let count: usize = weight.iter().product::<i64>() as usize;
        let data: Vec<f32> = (0..count)
            .map(|i| ((i * 37 % 101) as f32 - 50.0) / 500.0)
            .collect();
        let w = tensor("W", weight, &data);

        let mut graph = Vec::new();
        bytes_field(1, &node(&["X", "W"], &["Y"], "conv", "Conv"), &mut graph);
        string_field(2, "cudnn_conv", &mut graph);
        bytes_field(5, &w, &mut graph); // initializer
        bytes_field(11, &value_info("X", input), &mut graph);
        bytes_field(12, &value_info("Y", output), &mut graph);

        let mut opset = Vec::new();
        varint_field(2, 17, &mut opset); // OpSetIdProto.version

        let mut model = Vec::new();
        varint_field(1, 8, &mut model); // ModelProto.ir_version
        string_field(2, "ai-daw-cudnn-test", &mut model); // producer_name
        bytes_field(8, &opset, &mut model); // opset_import
        bytes_field(7, &graph, &mut model); // graph
        model
    }
}

#[test]
fn conv_runs_on_cuda() {
    let Some(runtime_dir) = std::env::var_os("AIDAW_TEST_RUNTIME") else {
        eprintln!("skipped: AIDAW_TEST_RUNTIME not set");
        return;
    };
    std::env::set_var("AIDAW_ORT_RUNTIME", &runtime_dir);
    ai_daw_local_ai::backend::set_backend(ai_daw_local_ai::backend::BackendKind::Cuda12);

    // 241k samples x stride 480, the same order of magnitude as the real Conv; a 1x1
    // convolution may take a different cuDNN path and never load the sublibrary.
    let model = onnx::conv_model(
        &[1, 1, 241_440],
        &[961, 1, 1_920],
        &[1, 961, 500],
    );
    let path = PathBuf::from(std::env::temp_dir()).join("ai-daw-cudnn-conv.onnx");
    std::fs::write(&path, &model).expect("write test model");

    let mut session = ai_daw_local_ai::backend::session::build(
        &path,
        ai_daw_local_ai::backend::session::GraphRole::Svc,
    )
    .unwrap_or_else(|err| panic!("build session failed: {err}"));

    let (gpu, cpu) = ai_daw_local_ai::backend::session::take_session_stats();
    eprintln!("build stats: GPU {gpu} / CPU {cpu}");
    assert_eq!(cpu, 0, "the graph fell back to CPU; CUDA Conv was not exercised");

    let input = vec![0.25f32; 241_440];
    let outputs = session
        .run(ort::inputs![
            "X" => ort::value::Value::from_array(([1usize, 1, 241_440], input)).expect("input tensor")
        ])
        .unwrap_or_else(|err| panic!("Conv on CUDA failed: {err:?}"));

    let (shape, data) = outputs[0].try_extract_tensor::<f32>().expect("output tensor");
    eprintln!("output {:?} ({} values)", shape, data.len());
    assert_eq!(shape.as_ref(), &[1, 961, 500]);
    assert!(data.iter().all(|v| v.is_finite()), "output contains NaN/Inf");
}
