"""Print the inputs and outputs of ONNX models."""
import sys
import onnx

for path in sys.argv[1:]:
    m = onnx.load(path, load_external_data=False)
    def shape(v):
        return [d.dim_param or d.dim_value for d in v.type.tensor_type.shape.dim]
    print("==", path, "opset", [o.version for o in m.opset_import])
    for i in m.graph.input:
        print("  in ", i.name, onnx.TensorProto.DataType.Name(i.type.tensor_type.elem_type), shape(i))
    for o in m.graph.output:
        print("  out", o.name, onnx.TensorProto.DataType.Name(o.type.tensor_type.elem_type), shape(o))
    ops = {n.op_type for n in m.graph.node}
    print("  has Softmax:", "Softmax" in ops, "| nodes:", len(m.graph.node))
