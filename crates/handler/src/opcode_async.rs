use crate::opcode_cache::OpCodeCache;
use crate::opcode_optimizer::do_code_fusion;
use bytecode::Bytecode;
use once_cell::sync::Lazy;
use primitives::{B256, Bytes};
use std::sync::mpsc::{self, Sender};
use std::thread;

#[derive(Clone, Copy)]
enum OptimizeTaskType {
    Generate,
    // Delete,
}

/// 后台工作线程的发送端。
/// static CODE_FUSION_TX: Lazy<Sender<(B256, Bytecode)>> = Lazy::new(|| {
/// let (tx, tx) = mpsc::channel::<(B256, Bytecode)>()})
static CODE_FUSION_TX: Lazy<Sender<(OptimizeTaskType, B256, Bytes)>> = Lazy::new(|| {
    let (tx, rx) = mpsc::channel::<(OptimizeTaskType, B256, Bytes)>();
    // 后台线程：持续处理融合并写入缓存。
    thread::Builder::new()
        .name("opcode_fusion_worker".into())
        .spawn(move || {
            while let Ok((typ, hash, code)) = rx.recv() {
                match typ {
                    OptimizeTaskType::Generate => {
                        match do_code_fusion(&code) {
                            Ok(fused_vec) => {
                                let fused = Bytecode::new_raw(Bytes::from(fused_vec));
                                OpCodeCache::insert(&hash, fused);
                            },
                            Err(e) => print!("failed: {:?}\n", e),
                        }
                    }
                    // OptimizeTaskType::Delete => {
                    //     // 目前未实现删除逻辑，可在此扩展。
                    //     OpCodeCache::remove(&hash);
                    // }
                }
            }
        })
        .expect("spawn fusion worker");
    tx
});

/// 尝试从缓存获取；未命中则异步提交给后台线程处理，并立即返回原始 code。
// pub(crate) fn gen_or_rewrite_optimized_code(hash: &B256, code: Bytecode) -> (Bytecode, bool) {
//     match OpCodeCache::get(hash) {
//         Ok(existing) => (existing, true),
//         Err(_) => {
//             let _ = CODE_FUSION_TX.send((OptimizeTaskType::Generate, hash.clone(), code.clone()));
//             (code, false)
//         }
//     }
// }

pub(crate) fn get_optimized_code(hash: &B256) -> Option<Bytecode> {
    OpCodeCache::get(hash)
}

pub(crate) fn send_optimized_code(hash: &B256, code: &[u8]) {
    let _ = CODE_FUSION_TX.send((OptimizeTaskType::Generate, hash.clone(), Bytes::copy_from_slice(code)));
}
