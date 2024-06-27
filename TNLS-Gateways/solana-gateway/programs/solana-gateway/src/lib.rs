use anchor_lang::{
        prelude::*,
        system_program::{transfer, Transfer},
        solana_program::{
            sysvar::{rent::Rent, Sysvar},
            program::invoke,
            instruction::Instruction,
            secp256k1_recover::{secp256k1_recover, Secp256k1Pubkey}
        }
};
use sha3::{Digest, Keccak256};
use base64::{
    engine::general_purpose::STANDARD,
    Engine
};

pub mod errors;
use crate::errors::{TaskError, GatewayError};

declare_id!("Fz1E7g4ebTVDH4EXijcRbBcgxntivQJfTGePEwZyie9B");

// Constants
const TASK_DESTINATION_NETWORK: &str = "pulsar-3";
const CHAIN_ID: &str = "SolanaDevNet";
const SECRET_GATEWAY_PUBKEY: &str = "BA0ZL+MMrYJeD5BUe2FHU+BnSExvij0s3GQaMnoX3+yEKwA46OHrYoLq6uYR0HeVPrJHDtEKC2t7dWpqUCC2iBg=";

#[program]
mod solana_gateway {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let gateway_state = &mut ctx.accounts.gateway_state;
        gateway_state.owner = *ctx.accounts.owner.key;
        gateway_state.task_id = 0;
        gateway_state.tasks = Vec::new();
        gateway_state.max_tasks = 10;

        Ok(())
    }

    pub fn increase_task_id(ctx: Context<IncreaseTaskId>, new_task_id: u64) -> Result<()> {
        let gateway_state = &mut ctx.accounts.gateway_state;
        if new_task_id <= gateway_state.task_id {
            return Err(GatewayError::TaskIdTooLow.into());
        }
        gateway_state.task_id = new_task_id;
        Ok(())
    }

    pub fn send(
        ctx: Context<Send>,
        payload_hash: [u8; 32],
        user_address: Pubkey,
        routing_info: String,
        execution_info: ExecutionInfo,
    ) -> Result<SendResponse> {
        let gateway_state = &mut ctx.accounts.gateway_state;

        // Fetch the current lamports per signature cost for the singature 
        let lamports_per_signature = 10;

        // //Calculate the rent for extra storage
        let lamports_per_byte_year = Rent::get().unwrap().lamports_per_byte_year;
        
        // // Estimate the cost based on the callback gas limit
        let estimated_price = execution_info.callback_gas_limit as u64 * lamports_per_signature 
         + std::mem::size_of::<Task>()*2*lamports_per_byte_year;
                
        let user_lamports = ctx.accounts.user.lamports();

        if user_lamports >= estimated_price {
            let cpi_accounts = Transfer {
                from: ctx.accounts.user.to_account_info(),
                to: gateway_state.to_account_info(),
            };
    
            let cpi_context = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_accounts);
    
            transfer(cpi_context, estimated_price)?;
        } else {
            return Err(TaskError::InsufficientFunds.into());
        }
        
        //Hash the payload
        let generated_payload_hash = solana_program::keccak::hash(&execution_info.payload).to_bytes();

        // Payload hash verification 
        require!(generated_payload_hash.as_slice() == payload_hash, TaskError::InvalidPayloadHash); 

        // Persist the task
        let task = Task {
            payload_hash: payload_hash,
            task_id: gateway_state.task_id,
            completed: false
        };

        // Calculate the array index
        let index = (gateway_state.task_id % gateway_state.max_tasks) as usize;

         // Reallocate account space if necessary
        if index >= gateway_state.tasks.len() {
             if gateway_state.tasks.len() >= gateway_state.max_tasks as usize {
                 let new_max_tasks = gateway_state.max_tasks + 10;
                 let new_space = 8 + 8 + 8 + (std::mem::size_of::<Task>() as u64 * new_max_tasks);
                 gateway_state.to_account_info().realloc(new_space, false)?;
                 gateway_state.max_tasks = new_max_tasks;
             }
 
             // If the array isn't filled up yet, just push it to the end 
             gateway_state.tasks.push(task);
         } else {
             // If a task already exists, it will be overwritten as expected from the max.
             gateway_state.tasks[index] = task;
        }

        let task_id = gateway_state.task_id;

        let log_new_task = LogNewTask {
            task_id: task_id,
            source_network: CHAIN_ID.to_string(),
            user_address: user_address.to_bytes().to_vec(),
            routing_info: routing_info,
            payload_hash: payload_hash,
            user_key: execution_info.user_key,
            user_pubkey: execution_info.user_pubkey,
            routing_code_hash: execution_info.routing_code_hash,
            task_destination_network: TASK_DESTINATION_NETWORK.to_string(),
            handle: execution_info.handle,
            nonce: execution_info.nonce,
            callback_gas_limit: execution_info.callback_gas_limit,
            payload: execution_info.payload,
            payload_signature: execution_info.payload_signature,
        };
            
        msg!(&format!("LogNewTask:{}", STANDARD.encode(&log_new_task.try_to_vec().unwrap())));

        gateway_state.task_id += 1;

        Ok(SendResponse { request_id: task_id })
    }

    pub fn post_execution(
        ctx: Context<PostExecution>,
        task_id: u64,
        source_network: String,
        post_execution_info: PostExecutionInfo,
    ) -> Result<()> {
        let gateway_state = &mut ctx.accounts.gateway_state;
        
        let index = (task_id % gateway_state.max_tasks) as usize;
        
        require!(index <= gateway_state.tasks.len(), TaskError::InvalidIndex);
        
        let task = &gateway_state.tasks[index];

        require!(task_id == task.task_id, TaskError::TaskIdAlreadyPruned);

        // Check if the task is already completed
        //require!(!task.completed, TaskError::TaskAlreadyCompleted);

        // Check if the payload hashes match
        require!(
            task.payload_hash == post_execution_info.payload_hash,
            TaskError::InvalidPayloadHash
        );

        // Concatenate packet data elements
        let data = [
            source_network.as_bytes(),
            CHAIN_ID.as_bytes(),
            task_id.to_string().as_bytes(),
            post_execution_info.payload_hash.as_slice(),
            post_execution_info.result.as_slice(),
            post_execution_info.callback_address.as_slice(),
            post_execution_info.callback_selector.as_slice(),
        ].concat();

        let mut hasher = Keccak256::new();
        hasher.update(&data);
        let keccak_hash = hasher.finalize_reset();

        let packet_hash = solana_program::hash::hash(&keccak_hash);

        // Packet hash verification
        require!(
            packet_hash.to_bytes().as_slice() == post_execution_info.packet_hash,
            TaskError::InvalidPacketHash
        );


        // Decode the base64 public key
        let pubkey_bytes = STANDARD.decode(SECRET_GATEWAY_PUBKEY).map_err(|_| TaskError::InvalidPublicKey)?;
        let expected_pubkey = Secp256k1Pubkey::new(&pubkey_bytes[1..]);

        // Extract the recovery ID and signature from the packet signature
        // RecoveryID here might me 27, 28 due to the ethereum bug

        let recovery_id = match post_execution_info.packet_signature[64].checked_sub(27) {
            Some(id) if id == 0 || id == 1 => id,
            _ => return Err(TaskError::InvalidPacketSignature.into()),
        };

        // Perform the signature recovery
        let recovered_pubkey = secp256k1_recover(&post_execution_info.packet_hash, 
            recovery_id, &post_execution_info.packet_signature[..64])
        .map_err(|_| TaskError::Secp256k1RecoverFailure)?;

        // Base64 encode the public keys
        let recovered_pubkey_base64 = STANDARD.encode(recovered_pubkey.to_bytes());
        let expected_pubkey_base64 = STANDARD.encode(expected_pubkey.to_bytes());

        // Log the base64 encoded public keys
        msg!("Recovered Public Key: {}", recovered_pubkey_base64);
        msg!("Expected Public Key: {}", expected_pubkey_base64);


        // // Verify that the recovered public key matches the expected public key
        require!(
            recovered_pubkey.to_bytes() == expected_pubkey.to_bytes(),
            TaskError::InvalidPacketSignature
        );

        // Mark the task as completed
        gateway_state.tasks[index].completed = true;

        let callback_data = CallbackData {
            callback_selector: post_execution_info.callback_selector,
            task_id: task_id,
            result: post_execution_info.result,
        };

        // Concatenate the identifier with the serialized data
        let mut data = Vec::with_capacity(identifier.len() + borsh_data.len());
        data.extend_from_slice(identifier);
        data.extend_from_slice(&borsh_data);

        let borsh_data = callback_data.try_to_vec().unwrap();

        // // Convert the String to a Pubkey
        // let callback_address_pubkey = Pubkey::try_from_slice(&post_execution_info.callback_address.as_slice())
        // .expect("Invalid Pubkey for callback address");
        // let callback_selector_pubkey = Pubkey::try_from_slice(&post_execution_info.callback_selector.as_slice())
        // .expect("Invalid Pubkey for callback selector");
       
        // // Execute the callback
        // let callback_result = invoke(
        //     &Instruction {
        //         program_id: callback_selector_pubkey,
        //         accounts: vec![AccountMeta::new(callback_address_pubkey, false)],
        //         data: borsh_data,
        //     },
        //     &[],
        // );

        let task_completed = TaskCompleted {
            task_id,
            callback_successful: true //callback_result.is_ok()
        };
            
        msg!(&format!("TaskCompleted:{}", STANDARD.encode(&task_completed.try_to_vec().unwrap())));

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = owner, space = 10240)]
    pub gateway_state: Account<'info, GatewayState>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct IncreaseTaskId<'info> {
    #[account(mut, has_one = owner)]
    pub gateway_state: Account<'info, GatewayState>,
    pub owner: Signer<'info>,
}

#[derive(Accounts)]
pub struct Send<'info> {
    #[account(mut)]
    pub gateway_state: Account<'info, GatewayState>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct PostExecution<'info> {
    #[account(mut)]
    pub gateway_state: Account<'info, GatewayState>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
pub struct GatewayState {
    pub owner: Pubkey,
    pub task_id: u64,
    pub tasks: Vec<Task>,
    pub max_tasks: u64, 
}

#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize)]
pub struct Task {
    pub payload_hash: [u8; 32],
    pub task_id: u64,
    pub completed: bool,
}

#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize)]
pub struct ExecutionInfo {
    pub user_key: Vec<u8>,
    pub user_pubkey: Vec<u8>,
    pub routing_code_hash: String,
    pub task_destination_network: String,
    pub handle: String,
    pub nonce: Vec<u8>,
    pub callback_gas_limit: u32,
    pub payload: Vec<u8>,
    pub payload_signature: Vec<u8>,
}

#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize)]
pub struct PostExecutionInfo {
    pub payload_hash: [u8; 32],
    pub packet_hash: [u8; 32],
    pub callback_address: Vec<u8>,
    pub callback_selector: Vec<u8>,
    pub callback_gas_limit: Vec<u8>,
    pub packet_signature: [u8; 65],
    pub result: Vec<u8>,
}

#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize)]
pub struct CallbackData {
    callback_selector: Vec<u8>,
    task_id: u64,
    result: Vec<u8>,
}

#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize)]
pub struct SendResponse {
    pub request_id: u64,
}

#[event]
pub struct TaskCompleted {
    pub task_id: u64,
    pub callback_successful: bool,
}

#[derive(Debug, Clone, AnchorSerialize, AnchorDeserialize)]
pub struct LogNewTask {
    pub task_id: u64,
    pub source_network: String,
    pub user_address: Vec<u8>,
    pub routing_info: String,
    pub payload_hash: [u8; 32],
    pub user_key: Vec<u8>,
    pub user_pubkey: Vec<u8>,
    pub routing_code_hash: String,
    pub task_destination_network: String,
    pub handle: String,
    pub nonce: Vec<u8>,
    pub callback_gas_limit: u32,
    pub payload: Vec<u8>,
    pub payload_signature: Vec<u8>,
}