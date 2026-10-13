# champions/

Generated champion files land here (`train-cli train --out champions`).

Everything in this directory except this README is git-ignored: champions are a
build artifact of a specific `sim_version` + `sensor_version` + `fitness_version`
trio, and stale ones are refused at load time anyway.
