# Tasked Futures

[![Build Status](https://travis-ci.org/erikjohnston/tasked-futures.svg?branch=master)](https://travis-ci.org/erikjohnston/tasked-futures)

Tasked futures conceptually allow a struct to act as an "executor" of child
futures. The main use is that the "executor" itself is passed as an extra
argument to the child futures when resolved.

See examples/ for example usage.
