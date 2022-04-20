# network_p2p

`CITA-Cloud`中[network微服务](https://github.com/cita-cloud/cita_cloud_proto/blob/master/protos/network.proto)的实现，基于`p2p`网络库[tentacle](https://github.com/nervosnetwork/tentacle)。

## 编译docker镜像
```
docker build -t citacloud/network_p2p .
```

## 使用方法

```
$ network -h
network 6.4.0
Rivtower Technologies <contact@rivtower.com>
network service

USAGE:
    network <SUBCOMMAND>

OPTIONS:
    -h, --help       Print help information
    -V, --version    Print version information

SUBCOMMANDS:
    help    Print this message or the help of the given subcommand(s)
    run     run this service
```

### network-run

运行`network`服务。

```
$ network run -h
network-run 
run this service

USAGE:
    network run [OPTIONS]

OPTIONS:
    -c, --config <CONFIG_PATH>    Chain config path [default: config.toml]
    -h, --help                    Print help information
    -l, --log <LOG_FILE>          log config path [default: network-log4rs.yaml]
```

参数：
1. 微服务配置文件。

    参见示例`example/config.toml`。

    其中：
    * `grpc_port` 为`gRPC`服务监听的端口号。
    * `port` 为节点网络的监听端口。
    * `peers` 为邻居节点的配置信息，其中只有一个`address`字段，配置邻居节点的网络地址，格式为[multi_address](https://multiformats.io/multiaddr/)。
2. 日志配置文件。

    参见示例`network-log4rs.yaml`。

    其中：

    * `level` 为日志等级。可选项有：`Error`，`Warn`，`Info`，`Debug`，`Trace`，默认为`Info`。
    * `appenders` 为输出选项，类型为一个数组。可选项有：标准输出(`stdout`)和滚动的日志文件（`journey-service`），默认为同时输出到两个地方。

```
$ network run -c example/config.toml -l network-log4rs.yaml 
2022-03-09T17:47:38.306022780+08:00 INFO network - grpc port of this service: 50000
2022-03-09T17:47:38.311991589+08:00 INFO network - Start network!
2022-03-09T17:47:38.312076023+08:00 INFO network - Start grpc server!
2022-03-09T17:47:38.313692735+08:00 INFO network::p2p - dial node(/dns4/127.0.0.1/tcp/40001)
2022-03-09T17:47:38.313770864+08:00 INFO network::p2p - dial node(/dns4/127.0.0.1/tcp/40002)
2022-03-09T17:47:38.313829770+08:00 INFO network::p2p - dial node(/dns4/127.0.0.1/tcp/40003)
```

## 设计


