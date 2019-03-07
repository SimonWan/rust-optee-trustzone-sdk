#![no_main]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use libc::{c_char, c_int, c_ulong, c_void, size_t, uint32_t};
use optee_utee;
use optee_utee::{trace_println, Error, ParamTypeFlags, Parameters, Result};
use optee_utee_sys::*;
use std::mem;

pub const AES128_KEY_BIT_SIZE: u32 = 128;
pub const AES128_KEY_BYTE_SIZE: u32 = AES128_KEY_BIT_SIZE / 8;
pub const AES256_KEY_BIT_SIZE: u32 = 256;
pub const AES256_KEY_BYTE_SIZE: u32 = AES256_KEY_BIT_SIZE / 8;

fn MESA_CreateEntryPoint() -> Result<()> {
    Ok(())
}

fn MESA_OpenSessionEntryPoint(_params: &mut Parameters, sess_ctx: *mut *mut c_void) -> Result<()> {
    unsafe {
        let sess: *mut aes_cipher =
            TEE_Malloc(mem::size_of::<aes_cipher>() as u32, 0) as *mut aes_cipher;
        if sess.is_null() {
            return Err(Error::from_raw_error(TEE_ERROR_OUT_OF_MEMORY));
        }
        (*sess).key_handle = TEE_HANDLE_NULL as *mut _;
        (*sess).op_handle = TEE_HANDLE_NULL as *mut _;
        *sess_ctx = sess as *mut c_void;
    }
    Ok(())
}

fn MESA_CloseSessionEntryPoint(sess_ctx: *mut *mut c_void) -> Result<()> {
    unsafe {
        let sess: *mut aes_cipher = sess_ctx as *mut aes_cipher;
        if ((*sess).key_handle) != TEE_HANDLE_NULL as *mut _ {
            TEE_FreeTransientObject((*sess).key_handle);
        }
        if ((*sess).op_handle) != TEE_HANDLE_NULL as *mut _ {
            TEE_FreeOperation((*sess).op_handle);
        }
        TEE_Free(sess as *mut c_void);
    }
    Ok(())
}

fn MESA_DestroyEntryPoint() -> Result<()> {
    Ok(())
}

fn MESA_InvokeCommandEntryPoint(
    sess_ctx: *mut c_void,
    cmd_id: u32,
    params: &mut Parameters,
) -> Result<()> {
    match cmd_id {
        TA_AES_CMD_PREPARE => {
            return alloc_resources(sess_ctx, params);
        }
        TA_AES_CMD_SET_KEY => {
            return set_aes_key(sess_ctx, params);
        }
        TA_AES_CMD_SET_IV => {
            return reset_aes_iv(sess_ctx, params);
        }
        TA_AES_CMD_CIPHER => {
            return cipher_buffer(sess_ctx, params);
        }
        _ => {
            return Err(Error::from_raw_error(TEE_ERROR_BAD_PARAMETERS));
        }
    }
}

#[repr(C)]
pub struct aes_cipher {
    pub algo: uint32_t,
    pub mode: uint32_t,
    pub key_size: uint32_t,
    pub op_handle: TEE_OperationHandle,
    pub key_handle: TEE_ObjectHandle,
}

pub fn ta2tee_algo_id(param: uint32_t, sess: *mut aes_cipher) -> Result<()> {
    unsafe {
        match param {
            TA_AES_ALGO_ECB => {
                (*sess).algo = TEE_ALG_AES_ECB_NOPAD;
                return Ok(());
            }
            TA_AES_ALGO_CBC => {
                (*sess).algo = TEE_ALG_AES_CBC_NOPAD;
                return Ok(());
            }
            TA_AES_ALGO_CTR => {
                (*sess).algo = TEE_ALG_AES_CTR;
                return Ok(());
            }
            _ => {
                return Err(Error::from_raw_error(TEE_ERROR_BAD_PARAMETERS));
            }
        }
    }
}

pub fn ta2tee_key_size(param: uint32_t, sess: *mut aes_cipher) -> Result<()> {
    unsafe {
        match param {
            AES128_KEY_BYTE_SIZE | AES256_KEY_BYTE_SIZE => {
                (*sess).key_size = param;
                return Ok(());
            }
            _ => {
                return Err(Error::from_raw_error(TEE_ERROR_BAD_PARAMETERS));
            }
        }
    }
}

pub fn ta2tee_mode_id(param: uint32_t, sess: *mut aes_cipher) -> Result<()> {
    unsafe {
        match param {
            TA_AES_MODE_ENCODE => {
                (*sess).mode = TEE_OperationMode::TEE_MODE_ENCRYPT as uint32_t;
                return Ok(());
            }
            TA_AES_MODE_DECODE => {
                (*sess).mode = TEE_OperationMode::TEE_MODE_DECRYPT as uint32_t;
                return Ok(());
            }
            _ => {
                return Err(Error::from_raw_error(TEE_ERROR_BAD_PARAMETERS));
            }
        }
    }
}

pub fn alloc_resources(sess_ctx: *mut c_void, params: &mut Parameters) -> Result<()> {
    params.check_type(
        ParamTypeFlags::ValueInput,
        ParamTypeFlags::ValueInput,
        ParamTypeFlags::ValueInput,
        ParamTypeFlags::None,
    )?;
    unsafe {
        let sess: *mut aes_cipher = sess_ctx as *mut aes_cipher;

        ta2tee_algo_id((*params.param_0.raw).value.a, sess)?;
        ta2tee_key_size((*params.param_1.raw).value.a, sess)?;
        ta2tee_mode_id((*params.param_2.raw).value.a, sess)?;

        if (*sess).op_handle != TEE_HANDLE_NULL as *mut _ {
            TEE_FreeOperation((*sess).op_handle);
        }

        let mut res: TEE_Result = TEE_AllocateOperation(
            &mut (*sess).op_handle,
            (*sess).algo,
            (*sess).mode,
            (*sess).key_size * 8,
        );

        'correct_handle: loop {
            if res != TEE_SUCCESS {
                trace_println!("[+] TA allocate operation failed.");
                (*sess).op_handle = TEE_HANDLE_NULL as *mut _;
                break 'correct_handle;
            }
            if (*sess).key_handle != TEE_HANDLE_NULL as *mut _ {
                TEE_FreeTransientObject((*sess).key_handle);
            }

            res = TEE_AllocateTransientObject(
                TEE_TYPE_AES,
                (*sess).key_size * 8,
                &mut (*sess).key_handle,
            );

            if res != TEE_SUCCESS {
                trace_println!("[+] TA allocate operation failed.");
                (*sess).key_handle = TEE_HANDLE_NULL as *mut _;
                break 'correct_handle;
            }

            let key: *mut c_char = TEE_Malloc((*sess).key_size, 0) as *mut _;

            if key.is_null() {
                res = TEE_ERROR_OUT_OF_MEMORY;
                trace_println!("[+] TA allocate key failed.");
                break 'correct_handle;
            }

            let mut attr = TEE_Attribute {
                attributeID: 0,
                content: content {
                    memref: Memref {
                        buffer: 0 as *mut _,
                        size: 0,
                    },
                },
            };
            TEE_InitRefAttribute(
                &mut attr,
                TEE_ATTR_SECRET_VALUE,
                key as *mut _,
                (*sess).key_size,
            );

            res = TEE_PopulateTransientObject((*sess).key_handle, &mut attr, 1);
            if res != TEE_SUCCESS {
                trace_println!("[+] TA populate transient object failed.");
                break 'correct_handle;
            }

            res = TEE_SetOperationKey((*sess).op_handle, (*sess).key_handle);
            if res != TEE_SUCCESS {
                trace_println!("[+] TA set operation key failed.");
                break 'correct_handle;
            }

            return Ok(());
        }
        trace_println!("[+] Error id is {}.", res);
        if ((*sess).op_handle) != TEE_HANDLE_NULL as *mut _ {
            TEE_FreeOperation((*sess).op_handle);
        }
        (*sess).op_handle = TEE_HANDLE_NULL as *mut _;

        if ((*sess).key_handle) != TEE_HANDLE_NULL as *mut _ {
            TEE_FreeTransientObject((*sess).key_handle);
        }
        (*sess).key_handle = TEE_HANDLE_NULL as *mut _;

        return Err(Error::from_raw_error(res));
    }
}

pub fn set_aes_key(sess_ctx: *mut c_void, params: &mut Parameters) -> Result<()> {
    params.check_type(
        ParamTypeFlags::MemrefInput,
        ParamTypeFlags::None,
        ParamTypeFlags::None,
        ParamTypeFlags::None,
    )?;

    unsafe {
        let sess: *mut aes_cipher = sess_ctx as *mut aes_cipher;
        let mut attr = TEE_Attribute {
            attributeID: 0,
            content: content {
                value: Value { a: 0, b: 0 },
            },
        };
        let key = (*params.param_0.raw).memref.buffer;
        let key_sz = (*params.param_0.raw).memref.size;

        if key_sz != (*sess).key_size {
            trace_println!("[+] Get wrong key size !\n");
            return Err(Error::from_raw_error(TEE_ERROR_BAD_PARAMETERS));
        }

        TEE_InitRefAttribute(&mut attr, TEE_ATTR_SECRET_VALUE, key, key_sz);
        TEE_ResetTransientObject((*sess).key_handle);
        let res = TEE_PopulateTransientObject((*sess).key_handle, &mut attr, 1);

        if res != TEE_SUCCESS {
            trace_println!("[+] TA set key failed!");
            return Err(Error::from_raw_error(res));
        } else {
            trace_println!("[+] TA set key success!");
            Ok(())
        }
    }
}

pub fn reset_aes_iv(sess_ctx: *mut c_void, params: &mut Parameters) -> Result<()> {
    params.check_type(
        ParamTypeFlags::MemrefInput,
        ParamTypeFlags::None,
        ParamTypeFlags::None,
        ParamTypeFlags::None,
    )?;

    unsafe {
        let sess: *mut aes_cipher = sess_ctx as *mut aes_cipher;
        let iv = (*params.param_0.raw).memref.buffer;
        let iv_sz = (*params.param_0.raw).memref.size;

        TEE_CipherInit((*sess).op_handle, iv, iv_sz);
    }
    trace_println!("[+] TA initial vectore reset done!");
    Ok(())
}

pub fn cipher_buffer(sess_ctx: *mut c_void, params: &mut Parameters) -> Result<()> {
    params.check_type(
        ParamTypeFlags::MemrefInput,
        ParamTypeFlags::MemrefOutput,
        ParamTypeFlags::None,
        ParamTypeFlags::None,
    )?;

    unsafe {
        let sess: *mut aes_cipher = sess_ctx as *mut aes_cipher;
        if (*params.param_1.raw).memref.size < (*params.param_0.raw).memref.size {
            return Err(Error::from_raw_error(TEE_ERROR_BAD_PARAMETERS));
        }

        if (*sess).op_handle == TEE_HANDLE_NULL as *mut _ {
            return Err(Error::from_raw_error(TEE_ERROR_BAD_STATE));
        }
        trace_println!("[+] TA tries to update ciphers!");

        let res = TEE_CipherUpdate(
            (*sess).op_handle,
            (*params.param_0.raw).memref.buffer,
            (*params.param_0.raw).memref.size,
            (*params.param_1.raw).memref.buffer,
            &mut (*params.param_1.raw).memref.size as *mut _,
        );
        if res == TEE_SUCCESS {
            return Ok(());
        } else {
            return Err(Error::from_raw_error(res));
        }
    }
}

const TA_FLAGS: uint32_t = TA_FLAG_EXEC_DDR;
const TA_STACK_SIZE: uint32_t = 2 * 1024;
const TA_DATA_SIZE: uint32_t = 32 * 1024;
const EXT_PROP_VALUE_1: &[u8] = b"Example of TA using an AES sequence\0";
const EXT_PROP_VALUE_2: uint32_t = 0x0010;

include!(concat!(env!("OUT_DIR"), "/user_ta_header.rs"));
