import sys

from ..cli import main

sys.exit(main(["bench", *sys.argv[1:]]))
