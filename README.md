# sisyphus

靶场应用

## 功能设计

- 资源的自动部署
  - 资源的定义
  - 资源的关系
  - 资源的管理
    - 新建
    - 部署
    - 监控
    - 回收

整个程序的后端是一个资源管理器，负责管理所有资源的生命周期，并在生命周期内与它们进行交互，来获取信息，通过`REST API`将结果暴露给外界

### 资源的定义

本系统范围内的资源就是应用程序，包括操作系统。**不包含任何硬件的虚拟**。对于不同级别的应用，按照它们的部署顺序和依赖关系分级。一般来说应用只分为两层， 即操作系统和任意运行在操作系统上的其它应用

#### 资源的属性

对于不同的应用资源，它们会自带不同的属性，但是有一个共有的资源结构

```rust
struct Resource {
    // 全局ID，自增
    id: u64,
    // 资源名称
    name: String,
    // 资源描述
    description: String,
    // 资源类型
    resource_type: ResourceType,
    // 资源组成形式，单一资源/组合资源
    resource_form: ResourceForm,
    // 资源等级，在创建组合资源时的判断基准，数字越小级别越高，高级别资源可以包含低级别资源
    level: u8,
    // 包含的其它资源
    contains: Option<Vec<Resource>>,
    // 创建时间
    create_datetime: DateTime<Local>,
    // 上一次修改时间
    last_update_datetime: DateTime<Local>,
    // 是否被删除
    deleted: bool,
    // 删除时间
    delete_datetime: Option<DateTime<Local>>,
}
```

#### 资源的生命周期

```rust
enum ResourceStatus {
    CREATED,
    DEPLOYED,
    PREPARED,
    USING,
    EXCEPTION,
    REVOKING,
    UNAVAILABLE,
    DELETED,
}
```

资源的生命周期以及对应的状态转变为

1. 创建资源，资源状态为`ResourceStatus::CREATED`
2. 后端接收到请求，资源被部署，状态变为`ResourceStatus::DEPLOYED`
3. 部署完成后需要有验证过程（主动检验/通知），得到部署状态成功，状态变为`ResourceStatus::PREPARED`
4. 后端接收到资源要被使用，修改状态为`ResourceStatus::USING`
   1. 使用过程中，如果后端对资源的监控得到异常信息（主动检验/通知），将修改资源状态为`ResourceStatus::EXCEPTION`
   2. 过一段时间/人工干预之后，根据监控得到的信息修改资源状态为`ResourceStatus::USING`
5. 后端接收到资源使用完毕的信息，执行回收资源/收集结果操作，将资源状态修改为`ResourceStatus::REVOKING`
6. 删除资源之前要将资源状态改为`ResourceStatus::UNAVAILABLE`
7. 删除资源后资源状态变为`ResourceStatus::DELETED`

### 资源的关系

资源直接的关系有包含和并列两种。包含关系发生在操作系统和应用之间。并列关系发生在操作系统+操作系统，应用+应用之间

多个资源建立关系之后可以作为一个单体资源，它们的行为关系遵循它们的部署关系

包含关系的部署顺序为，先部署高级别资源，再通过将低级别资源物理传输到高级别资源之后，进行部署。对于并列关系的资源，每个资源有**部署序列号**，根据序列号决定资源的部署顺序，如果序列号相同则表示可以同时部署

根据以上描述，一个资源的集合可以以一个节点为多叶子节点的高为2的树的链表表示，基于目前可以想到的，操作系统和应用的二层结构，则每颗树的根节点为操作系统，叶子节点为其上部署的应用，链表的顺序由它们的部署序列号的排序决定。对于整个资源组部署将是一个链表的顺序遍历和多叉树的广度优先遍历操作

### 资源的管理

通过资源管理器进程来对所有资源进行管理