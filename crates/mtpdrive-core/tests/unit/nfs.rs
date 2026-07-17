use super::*;

#[test]
fn names_are_safe_and_suffixes_are_short() {
    assert_eq!(safe_component("Pixel/Pro"), "Pixel_Pro");
    assert_eq!(serial_suffix("ABCDEF12"), "EF12");
}

#[test]
fn handles_are_stable_and_reject_other_filesystems() {
    let table = HandleTable::new();
    let handle = NodeHandle::Storage {
        device_key: "phone".into(),
        generation: 1,
        storage_id: 2,
    };
    let inode = table.intern(handle.clone());
    assert_eq!(inode, table.intern(handle.clone()));
    assert_eq!(table.resolve(&NfsFh3::new(inode, NFS_FSID)), Ok(handle));
    assert_eq!(
        table.resolve(&NfsFh3::new(inode, 99)),
        Err(Nfsstat3::Badhandle)
    );
}
