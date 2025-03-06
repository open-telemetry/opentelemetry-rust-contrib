# Local Setup for Testing `user_events` with *Multipass*

This directory contains `cloud-init.yaml`, a configuration file to set up an Ubuntu 24.04 virtual machine environment with essential tools required for running and validating user_events example contained in this repository. The setup includes enabling `user_events` and installing the `perf` tool, along with utilities for decoding `user_events` data and processing `proto` data.

## Prerequisites

1. **Virtualization Support**: Ensure that your machine supports virtualization. Virtualization options vary by operating system:
   - **Windows**: Requires Hyper-V, available on Windows 10 Pro or Enterprise and above.
   - **macOS**: Requires Apple Silicon (M1, M2) or Intel hardware with virtualization enabled in BIOS.
   - **Linux**: Ensure KVM or VirtualBox is available and enabled in your distribution. Run `lsmod | grep kvm` to verify if KVM is installed.

2. **Install Multipass**: Multipass is a tool for managing Ubuntu instances as lightweight virtual machines. Installation varies by OS, follow the instructions available in [official site](https://multipass.run/install).

   After installation, verify by running:

   ```bash
   PS> multipass --version
   ```

## Usage

1. **Launch a VM with cloud-init.yaml:**
    Use the following command to start a VM with the specified configuration:

    ```bash
    PS> multipass launch --name my-test-vm -m 6G -d 10G --cloud-init cloud-init.yaml
    Launched: my-test-vm
    ```

    This will take some time to create and configure the VM. Validate that the VM is created and running:

    ```bash
    PS> multipass list
    Name                    State             IPv4             Image
    my-test-vm              Running           172.27.162.116   Ubuntu 24.04 LTS    
    ```

2. **Login into the VM:** You should get the ubuntu bash shell after login:

    ```bash
    PS> multipass shell my-test-vm
    ..
    ubuntu@my-test-vm:~$
    ```

    Verify that the user_events in available and enabled:

    ```bash
    ubuntu@my-test-vm:~$ grep USER_EVENTS /boot/config-6.8.0-48-generic
    CONFIG_USER_EVENTS=y
    ```

3. **Run perf tool:** Invoke the perf tool to capture the user-events. Keep it running:

    ```bash
    ubuntu@my-test-vm:~$ sudo perf  record -e user_events:otlp_metrics
    ```

4. **Run user_events example** Open another shell, and build and run the opentelemetry-user-events-metrics exporter example:

    ```bash
    PS> multipass shell my-test-vm
    ubuntu@my-test-vm:~$ cd opentelemetry-rust-contrib/opentelemetry-user-events-metrics/ && cargo build --example basic-metrics --all-features
    ubuntu@my-test-vm:~$ $ sudo ~/opentelemetry-rust-contrib/target/debug/examples/basic-metrics
    Tracepoint registered successfully.
    ```

5. Terminate perf capture (Ctrl+C) after some time. It should show something like

    ```bash
    [ perf record: Woken up 1 times to write data ]
    [ perf record: Captured and wrote 0.175 MB perf.data (5 samples) ]
    ```

6. Convert perf data to json:

    ```bash
    ubuntu@my-test-vm:~$ sudo chmod uog+r ./perf.data
    ubuntu@my-test-vm:~$ perf-decode ./perf.data > perf.json
    ```

7. Ensure that `perf.json` contains something like:

    ```bash
    "./perf.data": [ { "n": "user_events:otlp_metrics", "protocol": 0, "version": "v0.19.00", "buffer": [ ... ], "meta": { "time": 816.790831600, "cpu": 0, "pid": 4957, "tid": 4958 } } ]
    ```

8. Convert perf json to OpenTelemetry format:

    ```bash
    ubuntu@my-test-vm:~$ source userevents-env/bin/activate
    (userevents-env) ubuntu@my-test-vm:~$ python3 decrypt_python.py perf.json
    ```

9. The output will look something like this:

```plaintext
resource_metrics {
  resource {
    attributes {
      key: "service.name"
      value {
        string_value: "metric-demo"
      }
    }
  }
  scope_metrics {
    scope {
      name: "user-event-test"
    }
    metrics {
      name: "counter_f64_test"
      description: "test_decription"
      unit: "test_unit"
      sum {
        data_points {
          start_time_unix_nano: 1731569774055345718
          time_unix_nano: 1731569834056820774
          as_double: 60
          attributes {
            key: "mykey1"
            value {
              string_value: "myvalue1"
            }
          }
          attributes {
            key: "mykey2"
            value {
              string_value: "myvalue2"
            }
          }
        }
        aggregation_temporality: AGGREGATION_TEMPORALITY_DELTA
        is_monotonic: true
      }
    }
    metrics {
      name: "counter_u64_test"
      description: "test_decription"
      unit: "test_unit"
      sum {
        data_points {
          start_time_unix_nano: 1731569774055358318
          time_unix_nano: 1731569834056835474
          as_int: 60
          attributes {
            key: "mykey1"
            value {
              string_value: "myvalue1"
            }
          }
          attributes {
            key: "mykey2"
            value {
              string_value: "myvalue2"
            }
          }
        }
        aggregation_temporality: AGGREGATION_TEMPORALITY_DELTA
        is_monotonic: true
      }
    }
    ...
  }
}
```

10. **Cleanup the VM:** Once tests are done, if required the VM can be stopped and/or deleted with below steps:

```bash
PS> multipass stop my-test-vm
```

And to delete:

```bash
PS> multipass delete my-test-vm
PS> multipass purge
```
