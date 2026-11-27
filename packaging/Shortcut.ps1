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
    [ComImport, Guid("000214F9-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface Link {
        void GetPath([Out,MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder value,int length,IntPtr findData,uint flags);
        void GetIDList(out IntPtr pidl); void SetIDList(IntPtr pidl);
        void GetDescription([Out,MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder value,int length);
        void SetDescription([MarshalAs(UnmanagedType.LPWStr)] string value);
        void GetWorkingDirectory([Out,MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder value,int length);
        void SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string value);
        void GetArguments([Out,MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder value,int length);
        void SetArguments([MarshalAs(UnmanagedType.LPWStr)] string value);
        void GetHotkey(out short key); void SetHotkey(short key);
        void GetShowCmd(out int cmd); void SetShowCmd(int cmd);
        void GetIconLocation([Out,MarshalAs(UnmanagedType.LPWStr)] System.Text.StringBuilder value,int length,out int index);
        void SetIconLocation([MarshalAs(UnmanagedType.LPWStr)] string value,int index);
        void SetRelativePath([MarshalAs(UnmanagedType.LPWStr)] string path,uint reserved);
        void Resolve(IntPtr hwnd,uint flags); void SetPath([MarshalAs(UnmanagedType.LPWStr)] string path);
    }
    static object NewLink() {
        return Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("00021401-0000-0000-C000-000000000046")));
    }
    public static void Create(string path,string target,string directory,string args) {
        object shell=NewLink();
        try {
            Link link=(Link)shell;
            link.SetPath(target); link.SetWorkingDirectory(directory); link.SetArguments(args);
            link.SetDescription("Agent Monitor dashboard");
            Store store=(Store)shell;
            var key=new Key {format=new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"),id=5};
            var v=new Variant {type=31,value=Marshal.StringToCoTaskMemUni("AgentMonitor.Dashboard")};
            try {store.SetValue(ref key,ref v);store.Commit();}
            finally {Marshal.FreeCoTaskMem(v.value);}
            ((System.Runtime.InteropServices.ComTypes.IPersistFile)shell).Save(path,true);
        } finally {Marshal.FinalReleaseComObject(shell);}
    }
    public static bool Matches(string path,string target,string directory,string args) {
        object shell=NewLink();
        try {
            ((System.Runtime.InteropServices.ComTypes.IPersistFile)shell).Load(path,0);
            Link link=(Link)shell;
            var t=new System.Text.StringBuilder(32768);var d=new System.Text.StringBuilder(32768);var a=new System.Text.StringBuilder(32768);
            link.GetPath(t,t.Capacity,IntPtr.Zero,4);link.GetWorkingDirectory(d,d.Capacity);link.GetArguments(a,a.Capacity);
            return String.Equals(t.ToString(),target,StringComparison.OrdinalIgnoreCase)
                && String.Equals(d.ToString(),directory,StringComparison.OrdinalIgnoreCase) && a.ToString()==args;
        } finally {Marshal.FinalReleaseComObject(shell);}
    }
}
'@
function New-DashboardLink($path,$target,$directory,$arguments) {
    [MonitorShortcut]::Create($path,$target,$directory,$arguments)
}
function Assert-DashboardLink($path,$target,$directory,$arguments) {
    $folder=(New-Object -ComObject Shell.Application).NameSpace([IO.Path]::GetDirectoryName($path))
    $item=$folder.ParseName([IO.Path]::GetFileName($path))
    if ($item.ExtendedProperty('System.AppUserModel.ID') -cne 'AgentMonitor.Dashboard' -or
        !([MonitorShortcut]::Matches($path,$target,$directory,$arguments))) {
        throw 'Shortcut readback failed.'
    }
}
