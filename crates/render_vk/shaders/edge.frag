#version 450

layout(location = 0) out vec4 out_color;

// Face-boundary outlines drawn on top of the shaded surface. A near-black
// constant matches the look you get out of FreeCAD/SolidWorks and reads well
// against any document background colour.
void main() {
    out_color = vec4(0.08, 0.08, 0.08, 1.0);
}
