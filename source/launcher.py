import subprocess
import os
import argparse



def launch_exe(exe_path, pfx_path, wine_bin):
    #exe_path: which windows exe to use (path)
    #pfx_path: path to the wine prefix (fake c: drive)
    #wine_bin: which actual wine thing to use (path)
    
    env = os.environ.copy()
    print(env)
    env["WINEPREFIX"] = pfx_path

    subprocess.run([wine_bin, exe_path], env=env)




def ensure_prefix(pfx_path, wine_bin):
    #Create w pfx if none   .
    
    if os.path.exists(pfx_path):
        print(f"Pfx exists : {pfx_path}")
        return

    print(f"New pfx : {pfx_path}")
    env = os.environ.copy()
    env["WINEPREFIX"] = pfx_path

    subprocess.run([wine_bin, "wineboot"], env=env)





if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Launch a Windows exe through Wine/Proton")
    parser.add_argument("--exe", required=True, help="Path to the .exe to launch")
    parser.add_argument("--prefix", required=True, help="Path to the Wine prefix")
    parser.add_argument("--wine", required=True, help="Path to the wine binary")

    args = parser.parse_args()

    exe = args.exe
    pfx = os.path.expanduser(args.prefix)
    wine = os.path.expanduser(args.wine)

    ensure_prefix(pfx, wine)
    launch_exe(exe, pfx, wine)