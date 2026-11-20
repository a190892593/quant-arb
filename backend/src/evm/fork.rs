//! REVM 分叉数据库：按区块号从 JSON-RPC 读取账号/代码/存储。
//!
//! 设计约束：
//! - `DatabaseRef` 是同步接口，而 RPC 是异步的；在 Tokio 多线程运行时里用
//!   `block_in_place + Handle::block_on` 桥接，避免另起线程池；
//! - 账号信息（余额/nonce/代码）一次 JSON-RPC 批量取回；存储槽按需读取并缓存；
//! - 所有读取都带区块号，保证 M2 使用的是“报价区块”的状态，而不是 latest。

use std::collections::HashMap;
use std::sync::Mutex;

use revm::bytecode::Bytecode;
use revm::database_interface::DBErrorMarker;
use revm::primitives::{Address, Bytes, B256, U256, keccak256};
use revm::state::AccountInfo;
use revm::DatabaseRef;

use super::RpcClient;

fn keccak_empty() -> B256 {
    keccak256([])
}

/// 分叉读取错误。
#[derive(Debug, Clone)]
pub struct ForkError(pub String);

impl std::fmt::Display for ForkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ForkError {}
impl DBErrorMarker for ForkError {}

#[derive(Default)]
struct Cache {
    accounts: HashMap<Address, Option<AccountInfo>>,
    storage: HashMap<(Address, U256), U256>,
    codes: HashMap<B256, Bytecode>,
    hashes: HashMap<u64, B256>,
}

/// 只读分叉数据库（对应某个固定区块）。
pub struct ForkDb {
    rpc: RpcClient,
    block: u64,
    /// 模拟调用方余额覆盖：只读模拟不需要真实资金，但 EVM 会校验 upfront cost。
    funded: HashMap<Address, U256>,
    code_overrides: HashMap<Address, Bytecode>,
    storage_overrides: HashMap<(Address, U256), U256>,
    cache: Mutex<Cache>,
}

impl ForkDb {
    pub fn new(rpc: RpcClient, block: u64) -> Self {
        Self {
            rpc,
            block,
            funded: HashMap::new(),
            code_overrides: HashMap::new(),
            storage_overrides: HashMap::new(),
            cache: Mutex::new(Cache::default()),
        }
    }

    /// 给某个地址注入模拟余额（不写入链，仅本次分叉）。
    pub fn fund(mut self, address: Address, balance: U256) -> Self {
        self.funded.insert(address, balance);
        self
    }

    /// 注入合约代码（仅本次分叉，不写链）。
    pub fn set_code(&mut self, address: Address, code: Vec<u8>) {
        let bytecode = Bytecode::new_raw(Bytes::from(code));
        let hash = keccak256(bytecode.original_bytes());
        self.code_overrides.insert(address, bytecode.clone());
        self.cache.lock().unwrap().codes.insert(hash, bytecode);
    }

    /// 注入存储槽（仅本次分叉），用于模拟余额/授权状态。
    pub fn set_storage(&mut self, address: Address, slot: U256, value: U256) {
        self.storage_overrides.insert((address, slot), value);
        self.cache.lock().unwrap().storage.insert((address, slot), value);
    }

    fn block_on<T>(&self, fut: impl std::future::Future<Output = Result<T, String>>) -> Result<T, ForkError> {
        // DatabaseRef 在 EVM 执行线程内被调用；Tokio 多线程运行时允许 block_in_place。
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| handle.block_on(fut)).map_err(ForkError)
    }

    fn hex_to_u256(s: &str) -> U256 {
        let t = s.trim_start_matches("0x");
        if t.is_empty() {
            return U256::ZERO;
        }
        let mut out = U256::ZERO;
        for chunk in t.as_bytes().chunks(16) {
            let part = std::str::from_utf8(chunk).unwrap_or("0");
            let v = u64::from_str_radix(part, 16).unwrap_or(0);
            let shift = (chunk.len() * 4) as usize;
            out = (out << shift) | U256::from(v);
        }
        out
    }

    fn fetch_account(&self, address: Address) -> Result<Option<AccountInfo>, ForkError> {
        // 一次批量取回余额 / nonce / 代码，减少公共 RPC 往返。
        let addr = format!("{address:?}");
        let calls = vec![
            (
                "eth_getBalance",
                serde_json::json!([addr, format!("0x{:x}", self.block)]),
            ),
            (
                "eth_getTransactionCount",
                serde_json::json!([addr, format!("0x{:x}", self.block)]),
            ),
            (
                "eth_getCode",
                serde_json::json!([addr, format!("0x{:x}", self.block)]),
            ),
        ];
        let results = self.block_on(self.rpc.batch_raw(calls))?;
        let balance = results
            .first()
            .and_then(|r| r.as_ref().ok())
            .and_then(|v| v.as_str())
            .map(Self::hex_to_u256)
            .unwrap_or(U256::ZERO);
        let nonce = results
            .get(1)
            .and_then(|r| r.as_ref().ok())
            .and_then(|v| v.as_str())
            .map(Self::hex_to_u256)
            .map(|v| v.as_limbs()[0])
            .unwrap_or(0);
        let code_hex = results
            .get(2)
            .and_then(|r| r.as_ref().ok())
            .and_then(|v| v.as_str())
            .unwrap_or("0x");
        let code_bytes = super::abi::hex_decode(code_hex).unwrap_or_default();
        if balance.is_zero() && nonce == 0 && code_bytes.is_empty() {
            return Ok(None);
        }
        let code_hash = if code_bytes.is_empty() {
            keccak_empty()
        } else {
            keccak256(&code_bytes)
        };
        let info = AccountInfo {
            balance,
            nonce,
            code_hash,
            code: if code_bytes.is_empty() {
                None
            } else {
                Some(Bytecode::new_raw(Bytes::from(code_bytes)))
            },
            ..Default::default()
        };
        Ok(Some(info))
    }
}

impl DatabaseRef for ForkDb {
    type Error = ForkError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        if let Some(code) = self.code_overrides.get(&address) {
            return Ok(Some(AccountInfo {
                balance: self.funded.get(&address).copied().unwrap_or(U256::ZERO),
                nonce: 0,
                code_hash: keccak256(code.original_bytes()),
                code: Some(code.clone()),
                ..Default::default()
            }));
        }
        if let Some(balance) = self.funded.get(&address).copied() {
            return Ok(Some(AccountInfo {
                balance,
                nonce: 0,
                code_hash: keccak_empty(),
                code: None,
                ..Default::default()
            }));
        }
        if let Some(hit) = self.cache.lock().unwrap().accounts.get(&address) {
            return Ok(hit.clone());
        }
        let info = self.fetch_account(address)?;
        self.cache.lock().unwrap().accounts.insert(address, info.clone());
        if let Some(i) = &info {
            if let Some(code) = &i.code {
                self.cache.lock().unwrap().codes.insert(i.code_hash, code.clone());
            }
        }
        Ok(info)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if let Some(code) = self.cache.lock().unwrap().codes.get(&code_hash) {
            return Ok(code.clone());
        }
        Ok(Bytecode::default())
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let key = (address, index);
        if let Some(v) = self.storage_overrides.get(&key) {
            return Ok(*v);
        }
        if let Some(v) = self.cache.lock().unwrap().storage.get(&key) {
            return Ok(*v);
        }
        let data = format!("0x{index:064x}");
        let body = self.block_on(async {
            self.rpc
                .call(
                    "eth_getStorageAt",
                    serde_json::json!([format!("{address:?}"), data, format!("0x{:x}", self.block)]),
                )
                .await
        })?;
        let value = body
            .as_str()
            .map(Self::hex_to_u256)
            .unwrap_or(U256::ZERO);
        self.cache.lock().unwrap().storage.insert(key, value);
        Ok(value)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        if let Some(h) = self.cache.lock().unwrap().hashes.get(&number) {
            return Ok(*h);
        }
        let body = self.block_on(async {
            self.rpc
                .call(
                    "eth_getBlockByNumber",
                    serde_json::json!([format!("0x{number:x}"), false]),
                )
                .await
        })?;
        let hex = body
            .get("hash")
            .and_then(|v| v.as_str())
            .unwrap_or("0x0");
        let hash = super::abi::hex_decode(hex)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .map(B256::from)
            .unwrap_or(B256::ZERO);
        self.cache.lock().unwrap().hashes.insert(number, hash);
        Ok(hash)
    }
}
