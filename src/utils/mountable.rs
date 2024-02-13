pub trait Mountable {
    fn mount(&self) -> bool;
    fn unmount(&self) -> bool;
    fn get_mount_path(&self) -> String;
}
