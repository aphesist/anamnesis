anamnesis @VERSION@ for windows (x86_64)
=========================================

a local-first second brain. everything stays on this machine unless you choose a model
that needs the network.

start
-----

double-click anamnesis-desktop.exe (the installers put it in the start menu as
"anamnesis"). it opens in its own window; closing the window stops anamnesis and saves
the brain. starting it again while it runs brings the window to the front.

the window uses microsoft edge webview2, which windows 11 and up-to-date windows 10
include. if it is missing, anamnesis says so and offers microsoft's installer:
https://go.microsoft.com/fwlink/p/?LinkId=2124703

while the window is open, mcp clients can also reach the brain over http at
http://127.0.0.1:7368/mcp (settings in the app show the exact snippet and its token).

the command line
----------------

anamnesis.exe is the same brain without a window, for mcp clients and headless use:

    anamnesis.exe mcp                  mcp over stdio, for claude code, codex, ...
    anamnesis.exe serve --open         serve and open the browser (as before)
    anamnesis.exe serve --demo --open  the demo brain
    anamnesis.exe --help               every command

files
-----

  anamnesis-desktop.exe              the app (the ui and the demo are inside it)
  anamnesis.exe                      the command line (mcp, headless serve, import, ...)
  onnxruntime.dll                    microsoft onnx runtime @ORT_VERSION@ (mit), for the
  onnxruntime_providers_shared.dll   on-device embedder; keep them next to the two .exe
  LICENSE.txt, NOTICE.txt            apache license 2.0 and the notices
  licenses\                          licences of the bundled third-party parts

onnxruntime.dll needs the microsoft visual c++ redistributable (x64), which most pcs
already have. without it anamnesis still runs, with a weaker offline embedder, and its log
says so. get it from https://aka.ms/vs/17/release/vc_redist.x64.exe

where your data goes
--------------------

  your brain         %APPDATA%\anamnesis\
  embedding models   %LOCALAPPDATA%\anamnesis\models\  (about 465 mb, downloaded on first run)
  api keys           windows credential manager, entries named "anamnesis"
  window and cache   %APPDATA%\ist.aphes.anamnesis\, %LOCALAPPDATA%\ist.aphes.anamnesis\
  the app's log      %APPDATA%\anamnesis\logs\anamnesis-desktop.log

this folder holds only the program: delete it to remove anamnesis. your brain stays in
%APPDATA%\anamnesis until you delete that too.

smartscreen
-----------

the files are not code-signed. if windows says "windows protected your pc", choose
more info, then run anyway.

more: https://github.com/aphesist/anamnesis/blob/main/docs/install.md
