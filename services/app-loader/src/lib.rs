pub mod api;
use api::*;

use xous_ipc::Buffer;
use num_traits::ToPrimitive;
use std::{error::Error, fmt};

// taken from the autogenerated app_autoload file
#[derive(Debug)]
pub enum AppDispatchError {
    IndexNotFound(usize),
}
impl Error for AppDispatchError {}
impl fmt::Display for AppDispatchError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AppDispatchError::IndexNotFound(app_index) => write!(f, "Index {} not found", app_index),
        }
    }
}

#[derive(Debug)]
pub enum LoadError {
    FailedToLoad(xous_ipc::String<64>),
}
impl Error for LoadError {}
impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LoadError::FailedToLoad(name) => write!(f, "Failed to load {}", name),
        }
    }
}

pub struct AppLoader {
    conn: xous::CID,
    status_cid: xous::CID,
    status_op: u32,
    menu_manager: gam::MenuMatic,
    num_apps: u32
}

impl AppLoader {
    pub fn new(xns: &xous_names::XousNames, status_cid: xous::CID, status_op: u32, app_menu_manager: gam::MenuMatic) -> Result<Self, xous::Error> {
	let conn = xns.request_connection_blocking(api::SERVER_NAME_APP_LOADER).expect("Can't connect to server!");
	Ok(AppLoader{ conn, status_cid, status_op, menu_manager: app_menu_manager, num_apps: 0 })
    }
    pub fn load_app(&mut self, name: xous_ipc::String<64>) -> Result<(), LoadError> {
	// add the item to the menu
	let item = gam::MenuItem {
	    name,
	    action_conn: Some(self.status_cid),
	    action_opcode: self.status_op,
	    action_payload: gam::MenuPayload::Scalar([self.num_apps, 0, 0, 0]),
	    close_on_select: true
	};
	self.menu_manager.add_item(item);
	self.num_apps += 1;

	let request = api::LoadAppRequest {
	    name
	};
	
	let buf = Buffer::into_buf(request).or(Err(LoadError::FailedToLoad(name)))?;
	buf.lend(self.conn, Opcode::LoadApp.to_u32().unwrap()).or(Err(LoadError::FailedToLoad(name)))?;

	Ok(())
    }
    pub fn app_index_to_name(&self, index: usize) -> Result<xous_ipc::String<64>, AppDispatchError> {
	let request = api::AppRequest {
	    index, auth: None
	};
	let mut buf = Buffer::into_buf(request).or(Err(AppDispatchError::IndexNotFound(index)))?;
	buf.lend_mut(self.conn, Opcode::FetchAppData.to_u32().unwrap()).or(Err(AppDispatchError::IndexNotFound(index)))?;
	match buf.to_original().unwrap() {
	    api::Return::Info(app) => {
		Ok(app.name)
	    },
	    api::Return::Failure => {
		Err(AppDispatchError::IndexNotFound(index))
	    }
	}
    }
    pub fn app_dispatch(&self, tokens: [u32; 4], index: usize) -> Result<(), AppDispatchError> {
	let request = api::AppRequest {
	    index, auth: Some(tokens)
	};
	let buf = Buffer::into_buf(request).or(Err(AppDispatchError::IndexNotFound(index)))?;
	buf.lend(self.conn, Opcode::DispatchApp.to_u32().unwrap()).or(Err(AppDispatchError::IndexNotFound(index)))?;
	Ok(())
    }
}
