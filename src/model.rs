#[derive(Debug, Clone)]
pub(crate) struct Header {
    pub(crate) unit_code: u8,
    pub(crate) device_capacity: u8,
    pub(crate) banner_offset: u32,
    pub(crate) arm9_offset: u32,
    pub(crate) arm9_entry: u32,
    pub(crate) arm9_ram: u32,
    pub(crate) arm9_size: u32,
    pub(crate) arm7_offset: u32,
    pub(crate) arm7_entry: u32,
    pub(crate) arm7_ram: u32,
    pub(crate) arm7_size: u32,
    pub(crate) fnt_offset: u32,
    pub(crate) fnt_size: u32,
    pub(crate) fat_offset: u32,
    pub(crate) fat_size: u32,
    pub(crate) title: String,
    pub(crate) game_code: String,
    pub(crate) maker_code: String,
}

#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) id: u32,
    pub(crate) path: String,
    pub(crate) start: u32,
    pub(crate) end: u32,
}
