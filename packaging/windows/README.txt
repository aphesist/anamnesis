anamnesis @VERSION@ for windows (x86_64)
=========================================

a local-first second brain. everything stays on this machine unless you choose a model
that needs the network.

start
-----

double-click anamnesis.exe. it serves on http://127.0.0.1:7368/ and opens your browser.
the console window shows the log; close it (or press ctrl+c in it) to stop anamnesis.
starting it again while it runs just opens the browser.

try the demo brain: open a terminal in this folder and run

    anamnesis.exe serve --demo --open

every command: anamnesis.exe --help

files
-----

  anamnesis.exe                      the program (the ui and the demo are inside it)
  onnxruntime.dll                    microsoft onnx runtime @ORT_VERSION@ (mit), for the
  onnxruntime_providers_shared.dll   on-device embedder; keep them next to anamnesis.exe
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

this folder holds only the program: delete it to remove anamnesis. your brain stays in
%APPDATA%\anamnesis until you delete that too.

smartscreen
-----------

the files are not code-signed. if windows says "windows protected your pc", choose
more info, then run anyway.

more: https://github.com/aphesist/anamnesis/blob/main/docs/install.md
