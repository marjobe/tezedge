// Copyright (c) SimpleStaking and Tezedge Contributors
// SPDX-License-Identifier: MIT

use std::cmp;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::convert::TryInto;
use std::sync::Arc;

use storage::{BlockHeaderWithHash, IteratorMode, OperationsMetaStorage, OperationsMetaStorageDatabase, OperationsStorage, OperationsStorageDatabase, StorageError};
use tezos_encoding::hash::BlockHash;
use tezos_messages::p2p::encoding::prelude::*;

use crate::collections::{BlockData, UniqueBlockData};

pub struct OperationsState {
    operations_storage: OperationsStorage,
    operations_meta_storage: OperationsMetaStorage,
    missing_operations_for_blocks: UniqueBlockData<MissingOperations>,
}

impl OperationsState {

    pub fn new(db: Arc<OperationsStorageDatabase>, meta_db: Arc<OperationsMetaStorageDatabase>) -> Self {
        OperationsState {
            operations_storage: OperationsStorage::new(db),
            operations_meta_storage: OperationsMetaStorage::new(meta_db),
            missing_operations_for_blocks: UniqueBlockData::new(),
        }
    }

    /// Process block header. This will create record in meta storage with
    /// unseen operations for the block header.
    ///
    /// If block header is not already present in storage, return `true`.
    ///
    /// If block is already present in storage return `false`.
    pub fn process_block_header(&mut self, block_header: &BlockHeaderWithHash) -> Result<bool, StorageError> {
        if !self.operations_meta_storage.contains(&block_header.hash)? {
            if block_header.header.validation_pass() > 0 {
                self.missing_operations_for_blocks.push(MissingOperations {
                    block_hash: block_header.hash.clone(),
                    validation_passes: (0..block_header.header.validation_pass())
                        .filter(|i| *i < std::i8::MAX.try_into().unwrap())
                        .map(|i| i.try_into().unwrap())
                        .collect(),
                    level: block_header.header.level()
                });
            }
            self.operations_meta_storage.put_block_header(block_header)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Process block operations. This will mark operations in store for the block as seen.
    ///
    /// If all block operations were processed return `true`.
    ///
    /// If there are still block operations to be processed return `false`.
    pub fn process_block_operations(&mut self, message: &OperationsForBlocksMessage) -> Result<bool, StorageError> {
        self.operations_storage.put_operations(message)?;
        self.operations_meta_storage.put_operations(message)?;
        self.operations_meta_storage.is_complete(message.operations_for_block().hash())
    }

    pub fn drain_missing_operations(&mut self, n: usize) -> Vec<MissingOperations> {
        (0..cmp::min(self.missing_operations_for_blocks.len(), n))
            .map(|_| self.missing_operations_for_blocks.pop().unwrap())
            .collect()
    }

    pub fn push_missing_operations<Q: Iterator<Item=MissingOperations>>(&mut self, missing_operations: Q) -> Result<(), StorageError>{
        for missing_operation in missing_operations {
            if !self.operations_meta_storage.is_complete(&missing_operation.block_hash)? {
                self.missing_operations_for_blocks.push(missing_operation);
            }
        }
        Ok(())
    }

    #[inline]
    pub fn has_missing_operations(&self) -> bool {
        !self.missing_operations_for_blocks.is_empty()
    }

    pub fn hydrate(&mut self) -> Result<(), StorageError> {
        for (key, value) in self.operations_meta_storage.iter(IteratorMode::Start)? {
            let (key, value) = (key?, value?);
            if !value.is_complete() {
                self.missing_operations_for_blocks.push(MissingOperations {
                    block_hash: key,
                    validation_passes: value.get_missing_validation_passes(),
                    level: value.level()
                });
            }
        }

        Ok(())
    }

}

#[derive(Clone, Debug)]
pub struct MissingOperations {
    pub block_hash: BlockHash,
    pub validation_passes: HashSet<i8>,
    pub level: i32
}

impl BlockData for MissingOperations {
    #[inline]
    fn block_hash(&self) -> &BlockHash {
        &self.block_hash
    }
}

impl PartialEq for MissingOperations {
    fn eq(&self, other: &Self) -> bool {
        self.level == other.level && self.block_hash == other.block_hash
    }
}

impl Eq for MissingOperations {}

impl PartialOrd for MissingOperations {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MissingOperations {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.level, &self.block_hash).cmp(&(other.level, &other.block_hash)).reverse()
    }
}

impl From<&MissingOperations> for Vec<OperationsForBlock> {
    fn from(ops: &MissingOperations) -> Self {
        ops.validation_passes
            .iter()
            .map(|vp| OperationsForBlock::new(ops.block_hash.clone(), *vp))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_operation_has_correct_ordering() {
        let mut heap = UniqueBlockData::new();
        heap.push(MissingOperations {
            level: 15,
            block_hash: vec![0, 0, 0, 1],
            validation_passes: HashSet::new()
        });
        heap.push(MissingOperations {
            level: 7,
            block_hash: vec![0, 0, 0, 9],
            validation_passes: HashSet::new()
        });
        heap.push(MissingOperations {
            level: 0,
            block_hash: vec![0, 0, 0, 4],
            validation_passes: HashSet::new()
        });
        heap.push(MissingOperations {
            level: 1,
            block_hash: vec![0, 0, 0, 5],
            validation_passes: HashSet::new()
        });

        let levels = (0..heap.len())
            .map(|_| heap.pop().unwrap())
            .map(|i| i.level)
            .collect::<Vec<i32>>();
        assert_eq!(vec![0, 1, 7, 15], levels)
    }
}