| Workload | Variant | Runs | LibMP µs per call | os.clock µs per call | KB per call | vs rbxtsc |
|---|---|---|---|---|---|---|
| loop | rbxtsc | 5 | 259.2 (250.6 to 265.6) | 259.1 (250.1 to 263.7) | 0.00 | 1.00x |
| construct | rbxtsc | 5 | 679.0 (672.8 to 692.9) | 679.2 (676.2 to 691.6) | GC inside calls | 1.00x |
| map | rbxtsc | 5 | 38.9 (37.0 to 39.5) | 38.7 (36.6 to 39.2) | 32.00 | 1.00x |
| vector | rbxtsc | 5 | 38.3 (36.4 to 39.5) | 38.3 (36.1 to 39.4) | 0.00 | 1.00x |
| pairs | rbxtsc | 5 | 92.4 (87.9 to 94.6) | 92.2 (88.9 to 93.9) | 156.00 | 1.00x |
| numeric | rbxtsc | 5 | 475.5 (439.2 to 480.6) | 475.7 (437.7 to 482.3) | 0.00 | 1.00x |
| loop | compiler | 5 | 421.1 (408.1 to 443.7) | 420.2 (407.7 to 442.6) | 0.00 | 1.62x |
| construct | compiler | 5 | 672.2 (661.8 to 692.0) | 670.7 (660.5 to 698.5) | GC inside calls | 0.99x |
| map | compiler | 5 | 38.7 (38.2 to 42.0) | 38.7 (38.1 to 42.5) | 32.00 | 0.99x |
| vector | compiler | 5 | 38.2 (37.5 to 39.2) | 37.9 (37.2 to 40.4) | 0.00 | 1.00x |
| pairs | compiler | 5 | 91.9 (87.8 to 106.7) | 91.3 (87.8 to 122.2) | 156.00 | 0.99x |
| numeric | compiler | 5 | 512.7 (492.7 to 527.5) | 512.4 (492.2 to 531.9) | 0.00 | 1.08x |
| loop | compiler-native | 5 | 198.0 (193.5 to 340.5) | 198.7 (193.2 to 341.4) | 0.00 | 0.76x |
| construct | compiler-native | 5 | 667.5 (627.8 to 1222.6) | 667.8 (627.0 to 1236.0) | GC inside calls | 0.98x |
| map | compiler-native | 5 | 21.6 (20.2 to 36.7) | 21.3 (19.9 to 36.5) | 32.00 | 0.55x |
| vector | compiler-native | 5 | 45.4 (43.1 to 85.8) | 45.3 (43.0 to 85.9) | 0.00 | 1.19x |
| pairs | compiler-native | 5 | 85.6 (71.3 to 152.0) | 84.0 (70.5 to 150.6) | 156.00 | 0.93x |
| numeric | compiler-native | 5 | 200.3 (196.6 to 242.4) | 200.2 (196.4 to 242.4) | 0.00 | 0.42x |

Runs 15, failed 0. CPU load median 25%, 10% to 63%.
Checksum disagreements: none.
