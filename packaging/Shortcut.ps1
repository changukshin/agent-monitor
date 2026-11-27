# Windows PowerShell 5.1. This property is written only to a newly staged .lnk.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class MonitorShortcut {
    [StructLayout(LayoutKind.Sequential, Pack=4)]
    struct Key { public Guid format; public uint id; }
    [StructLayout(LayoutKind.Explicit, Size=24)]
    struct Variant { [FieldOffset(0)] public ushort type; [FieldOffset(8)] public IntPtr value; }
    [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface Store {
        void GetCount(out uint n); void GetAt(uint i,out Key k);
        void GetValue(ref Key k,out Variant v); void SetValue(ref Key k,ref Variant v); void Commit();
    }
    [DllImport("shell32",CharSet=CharSet.Unicode,PreserveSig=false)]
    static extern void SHGetPropertyStoreFromParsingName(string path, IntPtr bind, uint flags, ref Guid iid, out Store store);
    public static void SetId(string path) {
        var key=new Key { format=new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), id=5 };
        Guid iid=typeof(Store).GUID; Store store;
        SHGetPropertyStoreFromParsingName(path,IntPtr.Zero,2,ref iid,out store);
        var v=new Variant { type=31, value=Marshal.StringToCoTaskMemUni("AgentMonitor.Dashboard") };
        try { store.SetValue(ref key,ref v); store.Commit(); }
        finally { Marshal.FreeCoTaskMem(v.value); Marshal.FinalReleaseComObject(store); }
    }
}
'@
function New-DashboardLink($path,$target,$directory,$arguments) {
    $link=(New-Object -ComObject WScript.Shell).CreateShortcut($path)
    $link.TargetPath=$target
    $link.WorkingDirectory=$directory
    $link.Arguments=$arguments
    $link.Description='Agent Monitor dashboard'
    $link.Save()
    [MonitorShortcut]::SetId($path)
}
function Assert-DashboardLink($path,$target,$directory,$arguments) {
    $link=(New-Object -ComObject WScript.Shell).CreateShortcut($path)
    $folder=(New-Object -ComObject Shell.Application).NameSpace([IO.Path]::GetDirectoryName($path))
    $item=$folder.ParseName([IO.Path]::GetFileName($path))
    if ($item.ExtendedProperty('System.AppUserModel.ID') -cne 'AgentMonitor.Dashboard' -or
        $link.TargetPath -ne $target -or $link.WorkingDirectory -ne $directory -or $link.Arguments -cne $arguments) {
        throw 'Shortcut readback failed.'
    }
}
